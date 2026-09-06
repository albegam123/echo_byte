#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct echo_audio_dsp echo_audio_dsp_t;

typedef struct {
    uint32_t sample_rate_hz;
    uint32_t frame_samples;
    uint32_t echo_tail_samples;
    uint32_t speaker_channels;
    int32_t noise_suppression_db;
    int32_t echo_suppression_db;
    int32_t echo_suppression_active_db;
    int32_t agc_target;
    int32_t agc_increment_db_per_second;
    int32_t agc_decrement_db_per_second;
    int32_t agc_max_gain_db;
    int32_t vad_start_probability;
    int32_t vad_continue_probability;
    bool enable_aec;
    bool enable_noise_suppression;
    bool enable_agc;
    bool enable_vad;
} echo_audio_dsp_config_t;

typedef struct {
    bool voice_active;
    int32_t speech_probability;
    int32_t agc_gain_db;
} echo_audio_dsp_stats_t;

enum {
    ECHO_AUDIO_DSP_OK = 0,
    ECHO_AUDIO_DSP_INVALID_ARGUMENT = -1,
    ECHO_AUDIO_DSP_OUT_OF_MEMORY = -2,
    ECHO_AUDIO_DSP_CONFIGURATION_FAILED = -3,
};

void echo_audio_dsp_default_config(echo_audio_dsp_config_t *config);

echo_audio_dsp_t *echo_audio_dsp_create(const echo_audio_dsp_config_t *config,
                                        int32_t *status);

void echo_audio_dsp_destroy(echo_audio_dsp_t *state);

/**
 * Process exactly config.frame_samples mono signed-16 capture samples.
 *
 * capture is the microphone signal. render is the time-aligned signal actually
 * sent to the speaker; it contains config.frame_samples *
 * config.speaker_channels interleaved samples, and may be NULL while nothing
 * is playing. output is always mono and contains config.frame_samples samples.
 * No allocation or locking occurs in this real-time function.
 */
int32_t echo_audio_dsp_process(echo_audio_dsp_t *state,
                               const int16_t *capture,
                               const int16_t *render,
                               int16_t *output,
                               size_t sample_count,
                               echo_audio_dsp_stats_t *stats);

void echo_audio_dsp_reset_aec(echo_audio_dsp_t *state);

uint32_t echo_audio_dsp_frame_samples(const echo_audio_dsp_t *state);

uint32_t echo_audio_dsp_speaker_channels(const echo_audio_dsp_t *state);

#ifdef __cplusplus
}
#endif
