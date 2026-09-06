# SpeexDSP source provenance

The files under `speexdsp/` are an unmodified, minimal subset of Xiph.org's
SpeexDSP repository at commit:

`7a158783df74efe7c2d1c6ee8363c1e695c71226`

Upstream: <https://github.com/xiph/speexdsp>

Included algorithms are the MDF acoustic echo canceller, preprocessor
(denoise, residual echo suppression, AGC and VAD), filterbank and Kiss FFT.
The original BSD 3-Clause license is preserved in
`speexdsp/licenses/SPEEXDSP-BSD-3-CLAUSE.txt`.

`config.h`, `speexdsp_config_types.h`, the ESP-IDF `CMakeLists.txt`, and the
`echo_audio_dsp.*` bridge are echo_byte porting files rather than upstream
files.
