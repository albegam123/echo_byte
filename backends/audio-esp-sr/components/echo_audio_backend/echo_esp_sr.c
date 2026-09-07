#include "echo_esp_sr.h"

#include <limits.h>
#include <stdlib.h>
#include <string.h>

#include "esp_afe_config.h"
#include "esp_afe_sr_models.h"
#include "esp_err.h"
#include "esp_heap_caps.h"
#include "esp_timer.h"
#include "esp_wn_models.h"
#include "model_path.h"

#define ECHO_ESP_SR_SAMPLE_RATE_HZ 16000
#define ECHO_ESP_SR_MODEL_FILTER "nihaoxiaozhi"

struct echo_esp_sr {
    srmodel_list_t *models;
    const esp_afe_sr_iface_t *iface;
    esp_afe_sr_data_t *afe;
    const char *model_name;
    int16_t *feed_buffer;
    uint32_t frame_samples;
    uint32_t speaker_channels;
    uint32_t internal_bytes;
    uint32_t psram_bytes;
    echo_esp_sr_mode_t mode;
};

static int16_t downmix_render(const int16_t *render,
                              size_t sample,
                              uint32_t channels) {
    if (render == NULL) {
        return 0;
    }
    if (channels == 1) {
        return render[sample];
    }

    int32_t sum = 0;
    for (uint32_t channel = 0; channel < channels; ++channel) {
        sum += render[sample * channels + channel];
    }
    int32_t mixed = sum / (int32_t)channels;
    if (mixed > INT16_MAX) {
        mixed = INT16_MAX;
    } else if (mixed < INT16_MIN) {
        mixed = INT16_MIN;
    }
    return (int16_t)mixed;
}

void echo_esp_sr_default_config(echo_esp_sr_config_t *config) {
    if (config == NULL) {
        return;
    }
    config->speaker_channels = 2;
    config->high_performance = false;
}

static void destroy_partial(echo_esp_sr_t *state) {
    if (state == NULL) {
        return;
    }
    if (state->afe != NULL && state->iface != NULL) {
        state->iface->destroy(state->afe);
    }
    free(state->feed_buffer);
    if (state->models != NULL) {
        esp_srmodel_deinit(state->models);
    }
    free(state);
}

echo_esp_sr_t *echo_esp_sr_create(const echo_esp_sr_config_t *config,
                                  int32_t *status) {
    if (status != NULL) {
        *status = ECHO_ESP_SR_INVALID_ARGUMENT;
    }
    if (config == NULL || config->speaker_channels == 0 ||
        config->speaker_channels > 2) {
        return NULL;
    }

    const size_t internal_before =
        heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    const size_t psram_before =
        heap_caps_get_free_size(MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);

    echo_esp_sr_t *state = calloc(1, sizeof(*state));
    if (state == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_OUT_OF_MEMORY;
        }
        return NULL;
    }
    state->speaker_channels = config->speaker_channels;

    state->models = esp_srmodel_init("model");
    if (state->models == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_MODEL_NOT_FOUND;
        }
        destroy_partial(state);
        return NULL;
    }
    state->model_name =
        esp_srmodel_filter(state->models, ESP_WN_PREFIX, ECHO_ESP_SR_MODEL_FILTER);
    if (state->model_name == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_MODEL_NOT_FOUND;
        }
        destroy_partial(state);
        return NULL;
    }

    afe_config_t *afe_config = afe_config_init(
        "MR",
        state->models,
        AFE_TYPE_FD,
        config->high_performance ? AFE_MODE_HIGH_PERF : AFE_MODE_LOW_COST);
    if (afe_config == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_OUT_OF_MEMORY;
        }
        destroy_partial(state);
        return NULL;
    }

    afe_config->aec_init = true;
    afe_config->aec_filter_length = 4;
    afe_config->ns_init = true;
    afe_config->afe_ns_mode = AFE_NS_MODE_WEBRTC;
    afe_config->ns_model_name = NULL;
    afe_config->vad_init = true;
    afe_config->vad_model_name = NULL;
    afe_config->wakenet_init = true;
    afe_config->wakenet_model_name = state->model_name;
    afe_config->wakenet_mode = DET_MODE_90;
    afe_config->agc_init = true;
    // ESP-SR's config checker selects WakeNet AGC whenever WakeNet is active.
    // This is an upstream pipeline constraint, not WebRTC AGC in that mode.
    afe_config->agc_mode = AFE_AGC_MODE_WEBRTC;
    afe_config->agc_compression_gain_db = 9;
    afe_config->agc_target_level_dbfs = 3;
    afe_config->afe_perferred_core = 1;
    afe_config->afe_perferred_priority = 5;
    afe_config->afe_ringbuf_size = 32;
    afe_config->memory_alloc_mode = AFE_MEMORY_ALLOC_MORE_PSRAM;

    afe_config_check(afe_config);
    if (!afe_config->aec_init || !afe_config->ns_init ||
        !afe_config->vad_init || !afe_config->wakenet_init ||
        !afe_config->agc_init) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_PIPELINE_UNAVAILABLE;
        }
        afe_config_free(afe_config);
        destroy_partial(state);
        return NULL;
    }
    state->iface = esp_afe_handle_from_config(afe_config);
    if (state->iface != NULL) {
        state->afe = state->iface->create_from_config(afe_config);
    }
    afe_config_free(afe_config);
    if (state->iface == NULL || state->afe == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_AFE_CREATE_FAILED;
        }
        destroy_partial(state);
        return NULL;
    }
    state->iface->print_pipeline(state->afe);

    const int sample_rate = state->iface->get_samp_rate(state->afe);
    const int feed_samples = state->iface->get_feed_chunksize(state->afe);
    const int fetch_samples = state->iface->get_fetch_chunksize(state->afe);
    const int feed_channels = state->iface->get_feed_channel_num(state->afe);
    if (sample_rate != ECHO_ESP_SR_SAMPLE_RATE_HZ || feed_samples <= 0 ||
        feed_samples != fetch_samples || feed_channels != 2) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_FRAME_MISMATCH;
        }
        destroy_partial(state);
        return NULL;
    }
    state->frame_samples = (uint32_t)feed_samples;
    state->feed_buffer = heap_caps_aligned_calloc(
        16,
        state->frame_samples * (uint32_t)feed_channels,
        sizeof(int16_t),
        MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    if (state->feed_buffer == NULL) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_OUT_OF_MEMORY;
        }
        destroy_partial(state);
        return NULL;
    }

    state->mode = ECHO_ESP_SR_MODE_FULL_DUPLEX;
    if (echo_esp_sr_set_mode(state, ECHO_ESP_SR_MODE_STANDBY) !=
        ECHO_ESP_SR_OK) {
        if (status != NULL) {
            *status = ECHO_ESP_SR_MODE_FAILED;
        }
        destroy_partial(state);
        return NULL;
    }

    const size_t internal_after =
        heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    const size_t psram_after =
        heap_caps_get_free_size(MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    state->internal_bytes = internal_before > internal_after
                                ? (uint32_t)(internal_before - internal_after)
                                : 0;
    state->psram_bytes =
        psram_before > psram_after ? (uint32_t)(psram_before - psram_after) : 0;

    if (status != NULL) {
        *status = ECHO_ESP_SR_OK;
    }
    return state;
}

void echo_esp_sr_destroy(echo_esp_sr_t *state) { destroy_partial(state); }

int32_t echo_esp_sr_set_mode(echo_esp_sr_t *state,
                             echo_esp_sr_mode_t mode) {
    if (state == NULL || state->iface == NULL || state->afe == NULL ||
        (mode != ECHO_ESP_SR_MODE_STANDBY &&
         mode != ECHO_ESP_SR_MODE_FULL_DUPLEX)) {
        return ECHO_ESP_SR_INVALID_ARGUMENT;
    }
    if (mode == state->mode) {
        return ECHO_ESP_SR_OK;
    }

    int aec_result;
    int wake_result;
    if (mode == ECHO_ESP_SR_MODE_STANDBY) {
        aec_result = state->iface->disable_aec(state->afe);
        wake_result = state->iface->enable_wakenet(state->afe);
    } else {
        wake_result = state->iface->disable_wakenet(state->afe);
        aec_result = state->iface->enable_aec(state->afe);
    }
    if (aec_result < 0 || wake_result < 0) {
        return ECHO_ESP_SR_MODE_FAILED;
    }
    if (state->iface->reset_buffer(state->afe) < 0) {
        return ECHO_ESP_SR_MODE_FAILED;
    }
    state->mode = mode;
    return ECHO_ESP_SR_OK;
}

uint32_t echo_esp_sr_frame_samples(const echo_esp_sr_t *state) {
    return state == NULL ? 0 : state->frame_samples;
}

int32_t echo_esp_sr_process(echo_esp_sr_t *state,
                            const int16_t *capture,
                            const int16_t *render,
                            int16_t *output,
                            size_t sample_count,
                            echo_esp_sr_stats_t *stats) {
    if (state == NULL || capture == NULL || output == NULL || stats == NULL ||
        sample_count != state->frame_samples) {
        return ECHO_ESP_SR_INVALID_ARGUMENT;
    }

    memset(stats, 0, sizeof(*stats));
    for (size_t sample = 0; sample < sample_count; ++sample) {
        state->feed_buffer[sample * 2] = capture[sample];
        state->feed_buffer[sample * 2 + 1] =
            downmix_render(render, sample, state->speaker_channels);
    }

    int64_t started = esp_timer_get_time();
    const int feed_result = state->iface->feed(state->afe, state->feed_buffer);
    stats->feed_time_us = (uint32_t)(esp_timer_get_time() - started);
    if (feed_result < 0) {
        return ECHO_ESP_SR_FEED_FAILED;
    }

    started = esp_timer_get_time();
    afe_fetch_result_t *result =
        state->iface->fetch_with_delay(state->afe, pdMS_TO_TICKS(24));
    stats->fetch_time_us = (uint32_t)(esp_timer_get_time() - started);
    if (result == NULL) {
        return ECHO_ESP_SR_FETCH_FAILED;
    }
    if (result->ret_value != ESP_OK) {
        memset(output, 0, sample_count * sizeof(int16_t));
        return ECHO_ESP_SR_OK;
    }
    if (result->data == NULL ||
        result->data_size != (int)(sample_count * sizeof(int16_t))) {
        return ECHO_ESP_SR_FETCH_FAILED;
    }

    memcpy(output, result->data, result->data_size);
    stats->voice_active = result->vad_state == VAD_SPEECH;
    stats->wake_detected = result->wakeup_state == WAKENET_DETECTED;
    stats->wake_word_index = result->wake_word_index;
    stats->input_volume_dbfs = result->data_volume;
    stats->output_samples = (uint32_t)(result->data_size / sizeof(int16_t));
    return ECHO_ESP_SR_OK;
}

void echo_esp_sr_memory(const echo_esp_sr_t *state,
                        echo_esp_sr_memory_t *memory) {
    if (memory == NULL) {
        return;
    }
    memory->internal_bytes = state == NULL ? 0 : state->internal_bytes;
    memory->psram_bytes = state == NULL ? 0 : state->psram_bytes;
}

const char *echo_esp_sr_model_name(const echo_esp_sr_t *state) {
    return state == NULL || state->model_name == NULL ? "unknown"
                                                      : state->model_name;
}
