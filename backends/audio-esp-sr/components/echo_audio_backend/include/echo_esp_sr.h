#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct echo_esp_sr echo_esp_sr_t;

typedef enum {
    ECHO_ESP_SR_MODE_STANDBY = 0,
    ECHO_ESP_SR_MODE_FULL_DUPLEX = 1,
} echo_esp_sr_mode_t;

typedef struct {
    uint32_t speaker_channels;
    bool high_performance;
} echo_esp_sr_config_t;

typedef struct {
    bool voice_active;
    bool wake_detected;
    int32_t wake_word_index;
    float input_volume_dbfs;
    uint32_t output_samples;
    uint32_t feed_time_us;
    uint32_t fetch_time_us;
} echo_esp_sr_stats_t;

typedef struct {
    uint32_t internal_bytes;
    uint32_t psram_bytes;
} echo_esp_sr_memory_t;

enum {
    ECHO_ESP_SR_OK = 0,
    ECHO_ESP_SR_INVALID_ARGUMENT = -1,
    ECHO_ESP_SR_OUT_OF_MEMORY = -2,
    ECHO_ESP_SR_MODEL_NOT_FOUND = -3,
    ECHO_ESP_SR_AFE_CREATE_FAILED = -4,
    ECHO_ESP_SR_FRAME_MISMATCH = -5,
    ECHO_ESP_SR_FEED_FAILED = -6,
    ECHO_ESP_SR_FETCH_FAILED = -7,
    ECHO_ESP_SR_MODE_FAILED = -8,
    ECHO_ESP_SR_PIPELINE_UNAVAILABLE = -9,
};

void echo_esp_sr_default_config(echo_esp_sr_config_t *config);

echo_esp_sr_t *echo_esp_sr_create(const echo_esp_sr_config_t *config,
                                  int32_t *status);

void echo_esp_sr_destroy(echo_esp_sr_t *state);

/**
 * Switch between local wake guarding and full-duplex conversation.
 *
 * Standby disables AEC and enables WakeNet. Full duplex enables AEC and
 * disables WakeNet. NS, AGC and VAD remain enabled in both modes.
 */
int32_t echo_esp_sr_set_mode(echo_esp_sr_t *state,
                             echo_esp_sr_mode_t mode);

/** Number of mono samples in one native ESP-SR feed/fetch frame. */
uint32_t echo_esp_sr_frame_samples(const echo_esp_sr_t *state);

/**
 * Feed one native frame and fetch one processed frame when available.
 *
 * capture and output contain frame_samples mono signed-16 samples. render is
 * optional and contains frame_samples * speaker_channels interleaved samples.
 * ESP-SR accepts one playback reference, so stereo render is downmixed using
 * a saturating arithmetic mean. ESP-SR is internally pipelined, so initial
 * calls may succeed with stats.output_samples == 0 and a zero-filled output.
 * No allocation occurs in this function.
 */
int32_t echo_esp_sr_process(echo_esp_sr_t *state,
                            const int16_t *capture,
                            const int16_t *render,
                            int16_t *output,
                            size_t sample_count,
                            echo_esp_sr_stats_t *stats);

void echo_esp_sr_memory(const echo_esp_sr_t *state,
                        echo_esp_sr_memory_t *memory);

const char *echo_esp_sr_model_name(const echo_esp_sr_t *state);

#ifdef __cplusplus
}
#endif
