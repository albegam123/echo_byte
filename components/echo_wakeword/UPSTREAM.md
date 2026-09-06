# microWakeWord source provenance

The inference runtime is assembled from these Apache-2.0 ESP-IDF components:

- `espressif/esp-tflite-micro` 1.3.7, upstream commit
  `6dcc4ae70959f409bea6795f1cece5a273b4a7a4`.
- `espressif/esp-nn` 1.1.2, upstream commit
  `596b08401a63da3a2e1b40868c442f582a99ae26`, explicitly pinned rather than
  accepting a moving transitive dependency.
- `esphome/esp-micro-speech-features` 1.2.3, upstream commit
  `351c4c69530f5a802da5433581c4863afadf0a00`.

The first bring-up model is the Apache-2.0 `Hey Jarvis` microWakeWord v2 model
from <https://github.com/esphome/micro-wake-word-models> at commit
`05b65922cc433c9df13e98e32a7fe520758c837e`. Its SHA-256 is
`21a7976add39ee24ec96c63d96b7aaa18e24d1d9824b963e451da8feb4b78b77`.
The model's manifest specifies a 0.97 cutoff, five-result sliding window,
10 ms feature step and 22,860-byte tensor arena.

`echo_wakeword.*` is an echo_byte implementation of the streaming inference
adapter. No ESPHome GPL runtime source is included or linked.
