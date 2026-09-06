idf_component_register(
    SRCS "echo_wakeword.cpp"
    INCLUDE_DIRS "include"
    REQUIRES
        espressif__esp-tflite-micro
        esphome__esp-micro-speech-features
        heap
    EMBED_FILES "models/hey_jarvis.tflite"
)

target_compile_options(${COMPONENT_LIB} PRIVATE
    -O3
    -fno-exceptions
    -fno-rtti
    -fno-threadsafe-statics
    -Wno-missing-field-initializers
)
