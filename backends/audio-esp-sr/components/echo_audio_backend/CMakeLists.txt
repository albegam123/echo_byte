idf_component_register(
    SRCS "echo_esp_sr.c"
    INCLUDE_DIRS "include"
    REQUIRES espressif__esp-sr esp_timer heap
)

target_compile_options(${COMPONENT_LIB} PRIVATE -O3)
