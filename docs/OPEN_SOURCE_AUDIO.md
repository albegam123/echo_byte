# echo_byte 开源语音前端

## 为什么不直接移植桌面版 openWakeWord

桌面版 openWakeWord 的运行链依赖 Python/ONNX Runtime，模型前面还有独立的
embedding 网络。它在 ESP32-S3 上并不是可直接部署的微控制器推理图。其官方说明
也指出，S3 处理一块 80 ms 音频可能需要数秒，并建议资源受限设备采用
microWakeWord。echo_byte 因此保留“完全开源离线唤醒”的目标，但选择专为 MCU
训练的 microWakeWord 流式模型格式。

## 固定依赖

| 功能 | 上游 | 固定版本/提交 | 许可证 |
| --- | --- | --- | --- |
| AEC、NS、AGC、VAD | `xiph/speexdsp` | `7a158783df74efe7c2d1c6ee8363c1e695c71226` | BSD-3-Clause |
| MCU 推理 | `espressif/esp-tflite-micro` | 1.3.7 | Apache-2.0 |
| 优化算子 | `espressif/esp-nn` | 1.1.2 / `596b08401a63da3a2e1b40868c442f582a99ae26` | Apache-2.0 |
| 音频特征 | `esphome/esp-micro-speech-features` | 1.2.3 | Apache-2.0 |
| Bring-up 模型 | `esphome/micro-wake-word-models` Hey Jarvis v2 | `05b65922cc433c9df13e98e32a7fe520758c837e` | Apache-2.0 |

第三方 ESP32-SpeexDSP 项目用于核对 ESP-IDF/Arduino 的构建经验，不作为源码
依赖。项目内 SpeexDSP 文件来自 Xiph 的可追踪快照，模型也保存了原始 manifest、
提交号和 SHA-256。

## PCM 契约

- 采样率：16,000 Hz。
- 算法帧：160 个 `i16` 单声道样本，即 10 ms。
- AEC capture：单声道麦克风。
- AEC render：与真正送入功放的数据时间对齐的双声道交错 PCM，320 个 `i16`。
- AEC/NS/AGC 输出及唤醒输入：单声道 `i16`。
- microWakeWord：30 ms 特征窗、10 ms 步长、40 个 int8 特征。

如果左右扬声器内容不同，两路都必须送进 AEC reference；提前混成单声道会丢掉
每个扬声器到麦克风的独立声学路径。SpeexDSP 因此使用
`speex_echo_state_init_mc(frame, tail, 1, 2)`。

## 实时与内存规则

SpeexDSP 的状态、FFT 工作区和 TFLM tensor arena 只在创建时分配。`process()`
热路径不调用分配器、不加锁；实例由 Rust `&mut` 独占并移动到固定音频任务。
TFLM arena 优先放内部 SRAM以保证延迟，内部 SRAM不足时才回退 PSRAM，并把实际
位置报告给自检日志。I2S DMA 缓冲始终必须位于 DMA 可访问的内部 SRAM。

SpeexDSP 当前使用浮点模式，因为 Xiph 上游的 AGC 只在浮点预处理器中实现。
ESP32-S3 有单精度 FPU，但是否满足 10 ms deadline 必须以板上测量为准；不能仅凭
“可以编译”宣称已经达到实时 3A。

## 到货后的硬件验收

1. 先分别录音和播放，确认 I2S 位宽、左右槽、极性和幅度。
2. 用准确的“实际播放 PCM”作为双声道 AEC reference，测量并补偿 DMA/功放延迟。
3. 连续记录 SpeexDSP 与 TFLM 每帧 p50/p95/p99 时间，任一 p99 不得超过 10 ms。
4. 测试近端讲话、远端 TTS、双讲、风扇噪声和静音，比较处理前后波形。
5. 用至少数小时背景音测误唤醒，并用多说话人样本测漏唤醒。
6. 在 Wi-Fi/BLE 共存、PSRAM 压力和连续全双工条件下做看门狗与栈余量测试。

Bring-up 的 `Hey Jarvis` 是英文通用模型，只用于验证端到端执行。正式产品应采集
目标用户与噪声条件，训练 `EchoByte` 专用模型，并重新确定阈值、滑窗与冷却时间。

## 首次无外设板测基线

ESP32-S3 运行在 240 MHz，Rust 主任务固定 Core 1，输入为零 PCM。SpeexDSP
100 帧平均 7.68 ms、最慢 8.18 ms；microWakeWord 110 个 PCM 帧平均 0.97 ms、
最慢 2.58 ms，期间完成 36 次模型推理，tensor arena 位于内部 SRAM。这证明两个
组件可独立运行。把完整 AEC 与唤醒推理强行串在每个 10 ms 周期时，推理帧最慢
10.42 ms，出现 36 次 deadline miss；因此运行状态必须互斥调度：

- 待机守门：喇叭静音，Speex 只做 NS/AGC/VAD（render 为 `None`，跳过 AEC），
  然后运行 microWakeWord。
- 全双工会话：运行双声道 reference AEC + NS/AGC/VAD，停止唤醒推理。

按上述互斥调度复测待机路径，110 帧平均 3.59 ms、最慢 5.13 ms、零 deadline
miss；全双工路径则由完整 SpeexDSP 的 7.68/8.18 ms 数据覆盖。两者均在当前
无外设、Wi-Fi 初始化前的合成基准中满足 10 ms deadline。

真实 I2S、Wi-Fi 中断和状态切换仍须用 standby/full-duplex 两条 pipeline 分别记录
deadline miss，不能用独立算法数据代替最终验收。
