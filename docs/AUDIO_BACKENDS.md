# 音频后端的编译期选择与评测契约

echo_byte 提供两个互斥的 Cargo feature：

| feature | 3A / VAD | 唤醒 | 默认模型 |
| --- | --- | --- | --- |
| `audio-open`（默认） | Xiph SpeexDSP AEC、NS、AGC、VAD | microWakeWord + TFLite Micro/ESP-NN | Hey Jarvis v2 |
| `audio-esp-sr` | ESP-SR AFE AEC、WebRTC NS/VAD、AFE AGC | WakeNet10 | 你好小智 |

Rust 在编译期要求恰好选择一个；同时启用或全部关闭都会 `compile_error!`。构建命令：

```bash
scripts/build_audio_backend.sh open
scripts/build_audio_backend.sh esp-sr
```

构建后可用同一入口烧录；ESP-SR 入口会自动追加模型分区：

```bash
scripts/flash_audio_backend.sh open /dev/ttyACM0
scripts/flash_audio_backend.sh esp-sr /dev/ttyACM0
```

等价的原始 Cargo 命令是：

```bash
cargo +esp build --release
cargo +esp build --release --no-default-features --features audio-esp-sr
```

两个底层 ESP-IDF 组件都会在首次 CMake 构建时生成静态库和 Rust bindings。这是
`esp-idf-sys` 当前对 Cargo feature/组件元数据缓存方式的限制。最终固件只引用所选
后端；链接器的 section GC 会删除另一后端的未引用对象。每次发布前仍用 `nm` 和
map 文件验证，不能仅凭 feature 名称推断最终体积。辅助脚本会将易被下一次构建
覆盖的 ELF 保存到 `target/audio-artifacts/<backend>/`。
该目录同时保存 `bootloader.bin` 与 `partition-table.bin`；不要只复制 ELF 到其他
目录后烧录，否则 `espflash` 无法发现配套的 32 MB 自定义分区表。

## 统一接口

两套实现都通过 `AudioFrontend` 输出相同的 `ProcessStats` 和 mono `i16` PCM：

- 采样率固定为 16 kHz；capture/output 都是一声道。
- render 是双声道交错的实际播放 PCM。
- `process_standby` 关闭 AEC、运行 NS/AGC/VAD 和本地唤醒。
- `process_full_duplex` 开启 AEC、运行 NS/AGC/VAD，并停止唤醒推理。
- ESP-SR 是内部任务流水线；启动或切换模式后的首批调用可能返回
  `output_ready=false`，调用成功但输出为静音，I2S/上传层不得把它当成有效 PCM。
- 初始化时分配状态和工作区；实时处理函数本身不扩容。
- Rust 用 `&mut self` 串行访问后端；句柄可以移动到专属音频任务，但不是 `Sync`。

原生帧长不同，必须按“处理时间 / 输入音频时长”比较 CPU，而不能直接比较单次
调用：SpeexDSP 使用 160 samples（10 ms），ESP-SR FD AFE 使用运行时查询到的
原生帧，ESP32-S3 2.5.3 当前预期为 512 samples（32 ms）。I2S 层到位后应使用
小 DMA 块连续采集，再在音频任务内组装后端原生帧。

ESP-SR AFE 当前只接受一个 playback reference。echo_byte 会把双声道 render 做
算术平均后送入闭源 AEC；SpeexDSP 则保留左右声道两条独立参考。这是算法能力差异，
后续 AEC 对比报告必须单列，不能表述为完全相同的双声道条件。

ESP-SR 2.5.3 在 WakeNet 启用时会自动禁用 WebRTC AGC，并把组合管线配置成
`AGC(WakeNet)`；这已由板端 pipeline 日志确认。因此这里只称“AFE AGC”，不能把
当前组合后端写成 WebRTC AGC。切换到 full-duplex 并停用 WakeNet 后的实际增益行为
仍要通过真实幅度阶跃与收敛测试验收，不能由配置字段推断。

`ProcessStats` 对后端无法提供的指标返回 `None`。例如 SpeexDSP 能报告语音概率和
AGC gain，ESP-SR AFE fetch 只公开 VAD 状态及 AGC 前的 dBFS，不应伪造可比较数值。

## ESP-SR 模型烧录

ESP-SR 依赖独立的 `model` 分区。脚本构建后先写应用，再写模型镜像：

```bash
espflash flash --port /dev/ttyACM0 \
  --flash-size 32mb \
  --bootloader target/audio-artifacts/esp-sr/bootloader.bin \
  --partition-table partitions.csv --partition-table-offset 0x8000 \
  --target-app-partition factory \
  target/audio-artifacts/esp-sr/echo_byte.elf
espflash write-bin --monitor --port /dev/ttyACM0 0x1220000 \
  target/audio-artifacts/esp-sr/srmodels.bin
```

只烧 ELF 不会写入 `srmodels.bin`，此时启动自检会明确报告模型分区缺失，但主配网
流程仍继续运行。`partitions.csv` 中 `model` 分区固定在 `0x1220000`，修改分区表后
必须同步更新命令，量产工具应根据 partition table 查询标签而不是硬编码偏移。

## 评测记录

同一块板、240 MHz、相同 PCM 流和相同 Wi-Fi/BLE 负载下，至少记录：

1. ELF loadable text/data、app image、模型镜像各自大小；
2. 初始化耗时、内部 SRAM 与 PSRAM 增量；
3. standby 与 full-duplex 的平均、p95、p99、最大处理时间及 deadline miss；
4. AEC ERLE、双讲衰减、NS 主观/客观质量、AGC 收敛与 VAD 起止延迟；
5. 每小时误唤醒、多人多距离漏唤醒和唤醒延迟。

零 PCM 启动自检只能证明组件初始化、调度和 ABI 正常，不能替代真实麦克风、扬声器
回采、房间脉冲响应和背景噪声测试。

## 许可证边界

开源后端的依赖和模型许可证见 `OPEN_SOURCE_AUDIO.md`。ESP-SR 2.5.3 的 Registry
许可证为 **ESPRESSIF MIT License**：免费授权仅限运行于 Espressif 产品。它不是
不带芯片限制的标准 MIT，发布物、SBOM 和商业评估中必须保留这一限制及上游版权
声明。ESP-SR 通过 Component Registry 下载，本仓库不复制它的预编译闭源库。
固定版本、上游 commit 与随固件分发的许可证正文见
`backends/audio-esp-sr/UPSTREAM.md` 和 `LICENSE-ESPRESSIF-MIT.txt`。

## 当前零 PCM 基线

2026-09-07 在 ESP32-S3 240 MHz、Wi-Fi 初始化前完成的首次板测如下。ESP-SR 已
剔除每次模式切换的 4 帧固定预热；数据是 Rust 适配器的 wall occupancy，AFE
算法在内部任务运行，因此不是完整 CPU time，不能直接与单任务 Speex 时间作最终
CPU 优劣结论。

| 后端/模式 | 原生帧 | 平均 | 最大 | deadline miss | 有效输出 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Open standby | 10 ms | 3.64 ms | 4.70 ms | 0/110 | 110/110 |
| Open full-duplex | 10 ms | 7.70 ms | 7.74 ms | 0/100 | 100/100 |
| ESP-SR standby | 32 ms | 13.25 ms | 14.91 ms | 0/110 | 110/110 |
| ESP-SR full-duplex | 32 ms | 8.31 ms | 9.24 ms | 0/100 | 100/100 |

ESP-SR 初始化 355 ms，AFE/WakeNet 增量约为内部 SRAM 34,396 bytes、PSRAM
971,924 bytes，WakeNet 模型镜像 795,276 bytes。开源后端统一实例初始化约 61 ms，
增量为内部 SRAM 99,012 bytes、PSRAM 79,372 bytes。当前 release 的链接符号检查：

- open ELF：loadable text 1,331,572、data 302,860 bytes；只含
  `echo_audio_dsp_create` / `echo_wakeword_create`。
- ESP-SR ELF：loadable text 2,358,760、data 292,084 bytes；只含
  `echo_esp_sr_create` / `echo_esp_sr_process`。

生成的 app image 分别为 1,621,952 与 2,638,368 bytes；后者还必须另加 WakeNet
模型镜像。ELF 文件保留调试段，不能把其磁盘文件大小当作 flash 占用。
