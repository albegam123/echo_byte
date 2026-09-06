#include "echo_audio_dsp.h"

#include <stdlib.h>
#include <string.h>

#include "speex/speex_echo.h"
#include "speex/speex_preprocess.h"

struct echo_audio_dsp {
    SpeexEchoState *echo;
    SpeexPreprocessState *preprocess;
    uint32_t frame_samples;
    uint32_t speaker_channels;
    bool aec_enabled;
};

static int configure_preprocessor(echo_audio_dsp_t *state,
                                  const echo_audio_dsp_config_t *config) {
    spx_int32_t enabled;
    spx_int32_t value;

    enabled = config->enable_noise_suppression;
    if (speex_preprocess_ctl(state->preprocess, SPEEX_PREPROCESS_SET_DENOISE,
                             &enabled) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->noise_suppression_db;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_NOISE_SUPPRESS, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }

    enabled = config->enable_agc;
    if (speex_preprocess_ctl(state->preprocess, SPEEX_PREPROCESS_SET_AGC,
                             &enabled) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->agc_target;
    if (speex_preprocess_ctl(state->preprocess, SPEEX_PREPROCESS_SET_AGC_TARGET,
                             &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->agc_increment_db_per_second;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_AGC_INCREMENT, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->agc_decrement_db_per_second;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_AGC_DECREMENT, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->agc_max_gain_db;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_AGC_MAX_GAIN, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }

    enabled = config->enable_vad;
    if (speex_preprocess_ctl(state->preprocess, SPEEX_PREPROCESS_SET_VAD,
                             &enabled) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->vad_start_probability;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_PROB_START, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->vad_continue_probability;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_PROB_CONTINUE, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }

    value = config->echo_suppression_db;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_ECHO_SUPPRESS, &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    value = config->echo_suppression_active_db;
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_ECHO_SUPPRESS_ACTIVE,
                             &value) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    if (speex_preprocess_ctl(state->preprocess,
                             SPEEX_PREPROCESS_SET_ECHO_STATE,
                             state->echo) != 0) {
        return ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
    }
    return ECHO_AUDIO_DSP_OK;
}

void echo_audio_dsp_default_config(echo_audio_dsp_config_t *config) {
    if (config == NULL) {
        return;
    }
    *config = (echo_audio_dsp_config_t) {
        .sample_rate_hz = 16000,
        .frame_samples = 160,
        .echo_tail_samples = 1600,
        .speaker_channels = 2,
        .noise_suppression_db = -20,
        .echo_suppression_db = -40,
        .echo_suppression_active_db = -15,
        .agc_target = 12000,
        .agc_increment_db_per_second = 12,
        .agc_decrement_db_per_second = -40,
        .agc_max_gain_db = 20,
        .vad_start_probability = 80,
        .vad_continue_probability = 65,
        .enable_aec = true,
        .enable_noise_suppression = true,
        .enable_agc = true,
        .enable_vad = true,
    };
}

echo_audio_dsp_t *echo_audio_dsp_create(const echo_audio_dsp_config_t *config,
                                        int32_t *status) {
    if (status != NULL) {
        *status = ECHO_AUDIO_DSP_INVALID_ARGUMENT;
    }
    if (config == NULL || config->sample_rate_hz == 0 ||
        config->frame_samples == 0 ||
        config->echo_tail_samples < config->frame_samples ||
        (config->speaker_channels != 1 && config->speaker_channels != 2) ||
        config->vad_start_probability < 0 ||
        config->vad_start_probability > 100 ||
        config->vad_continue_probability < 0 ||
        config->vad_continue_probability > 100 ||
        config->noise_suppression_db > 0 ||
        config->echo_suppression_db > 0 ||
        config->echo_suppression_active_db > 0 ||
        config->agc_target < 1 || config->agc_target > 32768) {
        return NULL;
    }

    echo_audio_dsp_t *state = calloc(1, sizeof(*state));
    if (state == NULL) {
        if (status != NULL) {
            *status = ECHO_AUDIO_DSP_OUT_OF_MEMORY;
        }
        return NULL;
    }
    state->frame_samples = config->frame_samples;
    state->speaker_channels = config->speaker_channels;
    state->aec_enabled = config->enable_aec;
    state->echo = speex_echo_state_init_mc((int) config->frame_samples,
                                           (int) config->echo_tail_samples,
                                           1,
                                           (int) config->speaker_channels);
    state->preprocess = speex_preprocess_state_init(
        (int) config->frame_samples, (int) config->sample_rate_hz);
    if (state->echo == NULL || state->preprocess == NULL) {
        echo_audio_dsp_destroy(state);
        if (status != NULL) {
            *status = ECHO_AUDIO_DSP_OUT_OF_MEMORY;
        }
        return NULL;
    }

    spx_int32_t sample_rate = (spx_int32_t) config->sample_rate_hz;
    if (speex_echo_ctl(state->echo, SPEEX_ECHO_SET_SAMPLING_RATE,
                       &sample_rate) != 0 ||
        configure_preprocessor(state, config) != ECHO_AUDIO_DSP_OK) {
        echo_audio_dsp_destroy(state);
        if (status != NULL) {
            *status = ECHO_AUDIO_DSP_CONFIGURATION_FAILED;
        }
        return NULL;
    }

    if (status != NULL) {
        *status = ECHO_AUDIO_DSP_OK;
    }
    return state;
}

void echo_audio_dsp_destroy(echo_audio_dsp_t *state) {
    if (state == NULL) {
        return;
    }
    if (state->preprocess != NULL) {
        speex_preprocess_state_destroy(state->preprocess);
    }
    if (state->echo != NULL) {
        speex_echo_state_destroy(state->echo);
    }
    free(state);
}

int32_t echo_audio_dsp_process(echo_audio_dsp_t *state,
                               const int16_t *capture,
                               const int16_t *render,
                               int16_t *output,
                               size_t sample_count,
                               echo_audio_dsp_stats_t *stats) {
    if (state == NULL || capture == NULL || output == NULL ||
        sample_count != state->frame_samples) {
        return ECHO_AUDIO_DSP_INVALID_ARGUMENT;
    }

    if (state->aec_enabled && render != NULL) {
        speex_echo_cancellation(state->echo, capture, render, output);
    } else if (output != capture) {
        memcpy(output, capture, sample_count * sizeof(*output));
    }

    int voice_active = speex_preprocess_run(state->preprocess, output);
    if (stats != NULL) {
        spx_int32_t probability = 0;
        spx_int32_t gain = 0;
        (void) speex_preprocess_ctl(state->preprocess,
                                    SPEEX_PREPROCESS_GET_PROB,
                                    &probability);
        (void) speex_preprocess_ctl(state->preprocess,
                                    SPEEX_PREPROCESS_GET_AGC_GAIN,
                                    &gain);
        stats->voice_active = voice_active != 0;
        stats->speech_probability = probability;
        stats->agc_gain_db = gain;
    }
    return ECHO_AUDIO_DSP_OK;
}

void echo_audio_dsp_reset_aec(echo_audio_dsp_t *state) {
    if (state != NULL && state->echo != NULL) {
        speex_echo_state_reset(state->echo);
    }
}

uint32_t echo_audio_dsp_frame_samples(const echo_audio_dsp_t *state) {
    return state == NULL ? 0 : state->frame_samples;
}

uint32_t echo_audio_dsp_speaker_channels(const echo_audio_dsp_t *state) {
    return state == NULL ? 0 : state->speaker_channels;
}
