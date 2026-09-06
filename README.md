# echo_byte

ESP32-S3-N32R16 无屏智能语音终端。当前第一阶段实现 Wi-Fi 自动连接和 BLE/AP 双轨配网；音频将在第二阶段加入。

完整阶段安排见 [`docs/ROADMAP.md`](docs/ROADMAP.md)。

## 配网行为

1. 启动后从 NVS 读取上一次验证成功的 Wi-Fi 凭据并尝试 STA 连接。
2. 无凭据或连接失败时，同时启动：
   - 开放热点 `Activate_CyberToy`；
   - `192.168.4.1` Captive Portal、通配 DNS；
   - BLE 外设 `EchoByte-Setup`。
3. Portal 或 BLE 第一份通过格式校验的凭据获胜。设备关闭 BLE、DNS、HTTP 和 AP，再切换至纯 STA。
4. 每份凭据会自动尝试三次，只有关联及 DHCP 成功后才写入 NVS；失败会记录
   ESP-IDF 断开原因，并自动恢复两种配网入口。

## BLE 协议

- Service: `7b3e0001-6d6f-4d65-9f20-6563686f6279`
- Credentials characteristic: `7b3e0002-6d6f-4d65-9f20-6563686f6279`
- Info characteristic: `7b3e0003-6d6f-4d65-9f20-6563686f6279`
- 写入 UTF-8 JSON：`{"ssid":"MyWifi","password":"secret123"}`

浏览器客户端位于 `web/ble_provision.html`。Web Bluetooth 要求 HTTPS 或 localhost，且浏览器本身需要支持 Web Bluetooth。

## 构建和烧录

```bash
source ~/export-esp.sh
cargo install ldproxy espflash --locked
cargo +esp build --release
espflash flash --monitor --port /dev/ttyACM0 \
  target/xtensa-esp32s3-espidf/release/echo_byte
```

本机用户需要拥有串口权限（通常加入 `dialout` 组）。

```bash
sudo usermod -aG dialout "$USER"
# 注销并重新登录后生效
```

当前配网固件使用 ESP-IDF 的 large single-app 分区表。仓库内的 `partitions.csv`
预留了第二阶段需要的 OTA、WakeNet 模型和数据区域，接入模型时再启用，避免当前
Rust 原生构建器对自定义 CSV 相对路径的处理差异影响第一阶段烧录。

## 安全说明

按产品规格，AP 是开放热点且 Portal 使用 HTTP，因此 Wi-Fi 密码在设备热点链路上没有应用层加密。量产前建议给 BLE 配网增加每台设备的 Proof-of-Possession，并重新评估开放 AP 要求。

## 第三方补丁

`vendor/esp32-nimble` 固定了 `esp32-nimble 0.12.0`，并修复 ESP-IDF 5.5
下 `BLEDevice::deinit_full()` 在 NimBLE 已释放后访问 GAP 状态导致的崩溃。
补丁原因与移除条件见 `vendor/esp32-nimble/PATCH.md`。
