# echo_byte

ESP32-S3-N32R16 无屏智能语音终端。第一阶段已实现 Wi-Fi 自动连接和
BLE/AP 双轨配网；第二阶段已提供开源与 ESP-SR 两套可编译选择的本地唤醒和 3A。

完整阶段安排见 [`docs/ROADMAP.md`](docs/ROADMAP.md)。

## 配网行为

1. NVS 最多保存 8 个已验证热点，按最近成功使用（MRU）记录顺序；旧版本保存的
   单个热点会在首次启动时自动迁移，不需要重新输入密码。
2. 启动或掉线重连耗尽后先扫描附近的 2.4 GHz 网络，只尝试当前可见的已知热点：
   信号较强者优先，同等信号按最近成功顺序；某个热点失败会继续尝试下一个。
3. 没有已知热点可见、所有候选都失败或尚无凭据时，同时启动：
   - 开放热点 `Activate_CyberToy`；
   - `192.168.4.1` Captive Portal、通配 DNS；
   - BLE 外设 `EchoByte-Setup`。
4. Portal 或 BLE 第一份通过格式校验的凭据获胜。设备关闭 BLE、DNS、HTTP 和 AP，
   再切换至纯 STA。
5. 新提交的凭据自动尝试三次，只有关联及 DHCP 成功后才加入 NVS 列表；同名热点
   更新密码并移到队首，第 9 个热点会淘汰最久未成功使用的一项。失败会记录
   ESP-IDF 断开原因，并自动恢复两种配网入口。

ESP32-S3 的 Wi-Fi 射频只支持 2.4 GHz。手机系统也不会把当前热点密码开放给网页、
BLE 外设或另一台设备读取，因此首次添加每个热点仍需用户明确提交一次密码；之后
设备会在保存的热点之间自动选择和故障转移。

## BLE 协议

- Service: `7b3e0001-6d6f-4d65-9f20-6563686f6279`
- Credentials characteristic: `7b3e0002-6d6f-4d65-9f20-6563686f6279`
- Info characteristic: `7b3e0003-6d6f-4d65-9f20-6563686f6279`
- Networks characteristic: `7b3e0004-6d6f-4d65-9f20-6563686f6279`
- 写入 UTF-8 JSON：`{"ssid":"MyWifi","password":"secret123"}`

Networks 特征采用分页协议：客户端先写入一个 `u8` 索引，再读取对应网络的 JSON；
索引超过扫描结果末尾时读取到 `null`。这避免热点较多时超过单个 GATT 属性限制。

浏览器客户端位于 `web/ble_provision.html`。网页连接 BLE 后会分页读取 ESP32 的
Wi-Fi 扫描结果并生成下拉列表，也保留隐藏网络的手动输入入口。Web Bluetooth
要求 HTTPS 或 localhost，且浏览器本身需要支持 Web Bluetooth。

本地已有 `~/scripts/cert/192.168.3.2+3.pem` 证书时，可在项目根目录启动：

```bash
python3 scripts/serve_ble_https.py
```

同一局域网内的 Android 手机使用 Chrome/Edge 打开
`https://192.168.3.2:8443/`。证书由本地 mkcert CA 签发，手机必须先信任对应的
`rootCA.pem`，否则页面不属于可信安全上下文，浏览器会禁用 Web Bluetooth。

## 构建和烧录

```bash
source ~/export-esp.sh
cargo install ldproxy espflash --locked
scripts/build_audio_backend.sh open
scripts/flash_audio_backend.sh open /dev/ttyACM0
```

本机用户需要拥有串口权限（通常加入 `dialout` 组）。

```bash
sudo usermod -aG dialout "$USER"
# 注销并重新登录后生效
```

当前启用 32 MB 自定义分区表，保留 factory、两个 6 MB OTA slot、8 MB ESP-SR
模型区和数据区。ESP-SR 后端还需单独烧录构建生成的模型镜像，完整命令见
[`docs/AUDIO_BACKENDS.md`](docs/AUDIO_BACKENDS.md)。

## 可选择的语音前端

默认后端是完全开源的 SpeexDSP + microWakeWord；也可在编译期选择乐鑫 ESP-SR
2.5.3 AFE + WakeNet10。两者共享 `AudioFrontend`、PCM 和统计契约，feature 互斥，
用于后续同板性能与效果评估。构建、烧录、帧长/双声道差异及许可证边界见
[`docs/AUDIO_BACKENDS.md`](docs/AUDIO_BACKENDS.md)。

- 3A 使用 Xiph SpeexDSP 的 BSD-3-Clause 官方源码固定快照：16 kHz、10 ms、
  单麦克风、双声道扬声器参考，包含 AEC、NS、AGC 和 VAD。浮点构建是刻意选择，
  因为上游 AGC 不存在于定点预处理器中。
- 唤醒使用 microWakeWord 模型格式、乐鑫 TensorFlow Lite Micro/ESP-NN 和
  Apache-2.0 的 micro-speech 特征生成器。首个板测模型是 `Hey Jarvis` v2；
  产品版会替换为专门训练的 `EchoByte` 模型。
- 两层都只在初始化时分配内存。每次音频处理不扩容、不加锁；Rust 端用唯一的
  `&mut` 所有权把各自的 C/C++ 状态限制在单一音频任务中。
- BOOT 键（GPIO0）经过 30 ms 消抖后，与语音检测统一产生 `WakeEvent`。
  开发板另一个标成 RST/EN 的按键是硬件复位，不是可供业务读取的 GPIO。

算法来源、参数与硬件验收项见
[`docs/OPEN_SOURCE_AUDIO.md`](docs/OPEN_SOURCE_AUDIO.md)。

## 安全说明

按产品规格，AP 是开放热点且 Portal 使用 HTTP，因此 Wi-Fi 密码在设备热点链路上没有应用层加密。量产前建议给 BLE 配网增加每台设备的 Proof-of-Possession，并重新评估开放 AP 要求。

## 第三方补丁

`vendor/esp32-nimble` 固定了 `esp32-nimble 0.12.0`，并修复 ESP-IDF 5.5
下 `BLEDevice::deinit_full()` 在 NimBLE 已释放后访问 GAP 状态导致的崩溃。
补丁原因与移除条件见 `vendor/esp32-nimble/PATCH.md`。
