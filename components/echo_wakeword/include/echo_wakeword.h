#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct echo_wakeword echo_wakeword_t;

typedef struct {
    uint32_t sample_rate_hz;
    uint8_t probability_cutoff;
    uint8_t sliding_window_size;
    uint16_t cooldown_feature_slices;
    size_t tensor_arena_size;
} echo_wakeword_config_t;

typedef struct {
    bool detected;
    bool inference_ran;
    bool arena_in_psram;
    uint8_t probability;
    uint8_t average_probability;
    uint32_t generated_feature_slices;
} echo_wakeword_stats_t;

enum {
    ECHO_WAKEWORD_OK = 0,
    ECHO_WAKEWORD_INVALID_ARGUMENT = -1,
    ECHO_WAKEWORD_OUT_OF_MEMORY = -2,
    ECHO_WAKEWORD_FRONTEND_FAILED = -3,
    ECHO_WAKEWORD_MODEL_INVALID = -4,
    ECHO_WAKEWORD_TENSOR_ALLOCATION_FAILED = -5,
    ECHO_WAKEWORD_INFERENCE_FAILED = -6,
};

void echo_wakeword_default_config(echo_wakeword_config_t *config);

echo_wakeword_t *echo_wakeword_create(const echo_wakeword_config_t *config,
                                      int32_t *status);

void echo_wakeword_destroy(echo_wakeword_t *state);

/**
 * Feed mono signed-16 PCM to the streaming feature frontend and model.
 *
 * The function accepts any non-empty sample count. It performs no allocation
 * and takes no locks, so a single instance can remain owned by the audio task.
 */
int32_t echo_wakeword_process(echo_wakeword_t *state,
                              const int16_t *samples,
                              size_t sample_count,
                              echo_wakeword_stats_t *stats);

/** Reset frontend, recurrent tensors, probability history and cooldown. */
int32_t echo_wakeword_reset(echo_wakeword_t *state);

const char *echo_wakeword_model_name(void);

#ifdef __cplusplus
}
#endif
