set(SPEEXDSP_DIR "${CMAKE_CURRENT_SOURCE_DIR}/speexdsp")

idf_component_register(
    SRCS
        "echo_audio_dsp.c"
        "${SPEEXDSP_DIR}/libspeexdsp/mdf.c"
        "${SPEEXDSP_DIR}/libspeexdsp/preprocess.c"
        "${SPEEXDSP_DIR}/libspeexdsp/fftwrap.c"
        "${SPEEXDSP_DIR}/libspeexdsp/filterbank.c"
        "${SPEEXDSP_DIR}/libspeexdsp/kiss_fft.c"
        "${SPEEXDSP_DIR}/libspeexdsp/kiss_fftr.c"
    INCLUDE_DIRS
        "include"
        "${SPEEXDSP_DIR}/include"
    PRIV_INCLUDE_DIRS
        "."
        "${SPEEXDSP_DIR}/libspeexdsp"
)

# Upstream SpeexDSP only implements AGC in its floating-point preprocessor.
# ESP32-S3 has a single-precision FPU; this mode provides the complete
# AEC + denoise + AGC chain instead of silently omitting the third A.
target_compile_definitions(${COMPONENT_LIB} PRIVATE
    HAVE_CONFIG_H=1
    FLOATING_POINT=1
    USE_KISS_FFT=1
    VAR_ARRAYS=1
    RELEASE=1
)
target_compile_options(${COMPONENT_LIB} PRIVATE
    -O3
    -Wno-unused-function
    -Wno-unused-variable
)
target_link_libraries(${COMPONENT_LIB} PRIVATE m)
