# echo_byte 开发路线

## 第一阶段：双轨自动配网（已完成）

- NVS 最多保存 8 个热点；兼容旧单热点格式迁移，成功后更新 MRU，超限 LRU 淘汰。
- 启动扫描当前可见的 2.4 GHz 已知热点，按信号强度及 MRU 顺序自动故障转移；
  全部失败才进入配网。
- 新凭据验证后提交、三次初连重试和在线断线重连。
- BLE NimBLE GATT 配网与 AP Captive Portal 同时运行。
- Web Bluetooth HTTPS 页面通过分页 GATT 特征读取 ESP32 扫描结果并生成热点下拉列表。
- 通配 DNS、常见系统联网探测路径和 Wi-Fi 扫描页面。
- 首个合法提交胜出；关闭 BLE、HTTP、DNS、AP 后切换纯 STA。
- 板端验收：Android/iOS/Windows Portal、WebBLE、错误密码恢复、断电重连。

## 第二阶段：双音频后端与离线唤醒（进行中）

- Xiph SpeexDSP AEC/NS/AGC/VAD 以窄 C ABI 接入 Rust；固定 16 kHz、10 ms 帧。
- AEC 输入为一声道麦克风和交错双声道播放参考，输出仍为一声道。
- microWakeWord v2 + TFLite Micro/ESP-NN + micro-speech frontend。
- `Hey Jarvis` 用于首次板测，后续训练并替换成 `EchoByte` 专用模型。
- ESP-SR 2.5.3 AFE（AEC/NS/AGC/VAD）+ WakeNet10 `你好小智` 通过窄 C ABI 接入。
- `audio-open` / `audio-esp-sr` 互斥 feature，共享 `AudioFrontend`、PCM 和性能统计契约。
- ESP-SR 原生 32 ms 帧及单 playback reference 与开源 10 ms/双 reference 的差异
  在评测报告中单列。
- GPIO0 BOOT 键和模型检测统一输出 `WakeEvent`；RST/EN 键只负责硬复位。
- 无音频器件时先运行零输入自检；器件到位后记录每帧耗时、内部 SRAM、PSRAM、
  误唤醒和漏唤醒数据。

## 第三阶段：灯效与音频硬件

- GPIO 5 的 WS2812B 状态驱动，先完成配网黄闪与联网绿灯。
- 在同一个 I2S 控制器上建立全双工 RX/TX，让 GPIO 1（BCLK）和 GPIO 2（WS）只由一套时钟配置驱动。
- GPIO 3 接 INMP441，GPIO 4 接 MAX98357A；完成录放音环回和提示音。
- 固定 DMA 缓冲位于内部 SRAM，大容量非 DMA 数据按用途放入 PSRAM。

## 第四阶段：本地语音状态机

- 将 I2S PCM 接入已经落地的 SpeexDSP 和 microWakeWord，验证任务优先级。
- 待机音频不出设备；只在唤醒后的会话窗口上传语音。
- 加入蓝色呼吸、紫色快闪及超时/错误灯效。

## 第五阶段：云端全双工会话

- WebSocket 信令、音频上下行拆分和有限状态机。
- 有界、无增长环形缓冲；明确欠载、过载和网络抖动策略。
- TLS、设备身份、服务端地址配置和远程日志。

## 第六阶段：可靠性与量产

- 长时压力、弱网、反复配网、掉电恢复和看门狗测试。
- 启用仓库中的 32 MB 自定义 OTA/模型分区，完成 A/B 升级和回滚。
- BLE Proof-of-Possession、凭据保护、恢复出厂入口和生产烧录流程。
