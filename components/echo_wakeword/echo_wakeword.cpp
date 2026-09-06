#include "echo_wakeword.h"

#include <algorithm>
#include <climits>
#include <cstring>
#include <new>

#include "esp_heap_caps.h"
#include "frontend.h"
#include "frontend_util.h"
#include "tensorflow/lite/core/c/common.h"
#include "tensorflow/lite/kernels/internal/tensor_ctypes.h"
#include "tensorflow/lite/micro/micro_allocator.h"
#include "tensorflow/lite/micro/micro_interpreter.h"
#include "tensorflow/lite/micro/micro_mutable_op_resolver.h"
#include "tensorflow/lite/micro/micro_resource_variable.h"
#include "tensorflow/lite/schema/schema_generated.h"

namespace {

constexpr uint32_t kSampleRateHz = 16000;
constexpr size_t kFeatureCount = 40;
constexpr uint8_t kDefaultProbabilityCutoff = 247;  // round(0.97 * 255)
constexpr uint8_t kDefaultSlidingWindowSize = 5;
constexpr uint16_t kDefaultCooldownFeatureSlices = 100;
constexpr size_t kDefaultTensorArenaSize = 22860;
constexpr size_t kVariableArenaSize = 2048;
constexpr int kMaximumResourceVariables = 20;

extern const uint8_t kHeyJarvisModelStart[]
    asm("_binary_hey_jarvis_tflite_start");
extern const uint8_t kHeyJarvisModelEnd[]
    asm("_binary_hey_jarvis_tflite_end");

bool register_streaming_ops(tflite::MicroMutableOpResolver<20> &resolver) {
    return resolver.AddCallOnce() == kTfLiteOk &&
           resolver.AddVarHandle() == kTfLiteOk &&
           resolver.AddReshape() == kTfLiteOk &&
           resolver.AddReadVariable() == kTfLiteOk &&
           resolver.AddStridedSlice() == kTfLiteOk &&
           resolver.AddConcatenation() == kTfLiteOk &&
           resolver.AddAssignVariable() == kTfLiteOk &&
           resolver.AddConv2D() == kTfLiteOk &&
           resolver.AddMul() == kTfLiteOk &&
           resolver.AddAdd() == kTfLiteOk &&
           resolver.AddMean() == kTfLiteOk &&
           resolver.AddFullyConnected() == kTfLiteOk &&
           resolver.AddLogistic() == kTfLiteOk &&
           resolver.AddQuantize() == kTfLiteOk &&
           resolver.AddDepthwiseConv2D() == kTfLiteOk &&
           resolver.AddAveragePool2D() == kTfLiteOk &&
           resolver.AddMaxPool2D() == kTfLiteOk &&
           resolver.AddPad() == kTfLiteOk &&
           resolver.AddPack() == kTfLiteOk &&
           resolver.AddSplitV() == kTfLiteOk;
}

void *allocate_arena(size_t size, bool *in_psram) {
    void *memory = heap_caps_aligned_alloc(
        16, size, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    if (memory != nullptr) {
        *in_psram = false;
        return memory;
    }

    memory = heap_caps_aligned_alloc(16, size, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (memory != nullptr) {
        *in_psram = true;
    }
    return memory;
}

int8_t quantize_feature(uint16_t feature) {
    // This is the integer equivalent of the microWakeWord training pipeline:
    // ((feature / 25.6) / 26.0) * 256 - 128.
    constexpr int32_t kValueScale = 256;
    constexpr int32_t kValueDivisor = 666;
    int32_t value =
        (static_cast<int32_t>(feature) * kValueScale + kValueDivisor / 2) /
            kValueDivisor +
        INT8_MIN;
    value = std::max<int32_t>(INT8_MIN, std::min<int32_t>(INT8_MAX, value));
    return static_cast<int8_t>(value);
}

}  // namespace

struct echo_wakeword {
    FrontendState frontend{};
    tflite::MicroMutableOpResolver<20> resolver{};
    tflite::MicroAllocator *variable_allocator{nullptr};
    tflite::MicroResourceVariables *resource_variables{nullptr};
    tflite::MicroInterpreter *interpreter{nullptr};
    uint8_t *tensor_arena{nullptr};
    uint8_t *variable_arena{nullptr};
    size_t tensor_arena_size{0};
    bool tensor_arena_in_psram{false};
    bool variable_arena_in_psram{false};
    bool frontend_ready{false};
    uint8_t probability_cutoff{0};
    uint8_t sliding_window_size{0};
    uint16_t cooldown_feature_slices{0};
    uint16_t cooldown_remaining{0};
    uint8_t current_stride_step{0};
    uint8_t probability_index{0};
    uint8_t probabilities[UINT8_MAX]{};
    uint8_t last_probability{0};
    uint8_t average_probability{0};
    uint32_t generated_feature_slices{0};
};

void echo_wakeword_default_config(echo_wakeword_config_t *config) {
    if (config == nullptr) {
        return;
    }
    std::memset(config, 0, sizeof(*config));
    config->sample_rate_hz = kSampleRateHz;
    config->probability_cutoff = kDefaultProbabilityCutoff;
    config->sliding_window_size = kDefaultSlidingWindowSize;
    config->cooldown_feature_slices = kDefaultCooldownFeatureSlices;
    config->tensor_arena_size = kDefaultTensorArenaSize;
}

echo_wakeword_t *echo_wakeword_create(const echo_wakeword_config_t *config,
                                      int32_t *status) {
    if (status != nullptr) {
        *status = ECHO_WAKEWORD_INVALID_ARGUMENT;
    }
    if (config == nullptr || config->sample_rate_hz != kSampleRateHz ||
        config->probability_cutoff == 0 || config->sliding_window_size == 0 ||
        config->tensor_arena_size == 0) {
        return nullptr;
    }

    auto *state = new (std::nothrow) echo_wakeword();
    if (state == nullptr) {
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_OUT_OF_MEMORY;
        }
        return nullptr;
    }

    state->probability_cutoff = config->probability_cutoff;
    state->sliding_window_size = config->sliding_window_size;
    state->cooldown_feature_slices = config->cooldown_feature_slices;
    state->cooldown_remaining = config->cooldown_feature_slices;
    state->tensor_arena_size = config->tensor_arena_size;

    FrontendConfig frontend_config{};
    FrontendFillConfigWithDefaults(&frontend_config);
    frontend_config.window.size_ms = 30;
    frontend_config.window.step_size_ms = 10;
    frontend_config.filterbank.num_channels = kFeatureCount;
    frontend_config.filterbank.lower_band_limit = 125.0f;
    frontend_config.filterbank.upper_band_limit = 7500.0f;
    frontend_config.noise_reduction.smoothing_bits = 10;
    frontend_config.noise_reduction.even_smoothing = 0.025f;
    frontend_config.noise_reduction.odd_smoothing = 0.06f;
    frontend_config.noise_reduction.min_signal_remaining = 0.05f;
    frontend_config.pcan_gain_control.enable_pcan = true;
    frontend_config.pcan_gain_control.strength = 0.95f;
    frontend_config.pcan_gain_control.offset = 80.0f;
    frontend_config.pcan_gain_control.gain_bits = 21;
    frontend_config.log_scale.enable_log = true;
    frontend_config.log_scale.scale_shift = 6;
    if (!FrontendPopulateState(&frontend_config, &state->frontend,
                               static_cast<int>(config->sample_rate_hz))) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_FRONTEND_FAILED;
        }
        return nullptr;
    }
    state->frontend_ready = true;

    const uintptr_t model_start =
        reinterpret_cast<uintptr_t>(kHeyJarvisModelStart);
    const uintptr_t model_end = reinterpret_cast<uintptr_t>(kHeyJarvisModelEnd);
    const size_t model_size = model_end - model_start;
    uint32_t root_offset = 0;
    if (model_end > model_start && model_size >= 8) {
        std::memcpy(&root_offset, kHeyJarvisModelStart, sizeof(root_offset));
    }
    if (model_end <= model_start || model_size < 8 ||
        std::memcmp(kHeyJarvisModelStart + 4, "TFL3", 4) != 0 ||
        root_offset >= model_size ||
        !register_streaming_ops(state->resolver)) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_MODEL_INVALID;
        }
        return nullptr;
    }
    const tflite::Model *model = tflite::GetModel(kHeyJarvisModelStart);
    if (model->version() != TFLITE_SCHEMA_VERSION) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_MODEL_INVALID;
        }
        return nullptr;
    }

    state->variable_arena = static_cast<uint8_t *>(
        allocate_arena(kVariableArenaSize, &state->variable_arena_in_psram));
    state->tensor_arena = static_cast<uint8_t *>(
        allocate_arena(state->tensor_arena_size,
                       &state->tensor_arena_in_psram));
    if (state->variable_arena == nullptr || state->tensor_arena == nullptr) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_OUT_OF_MEMORY;
        }
        return nullptr;
    }

    state->variable_allocator =
        tflite::MicroAllocator::Create(state->variable_arena,
                                       kVariableArenaSize);
    if (state->variable_allocator != nullptr) {
        state->resource_variables = tflite::MicroResourceVariables::Create(
            state->variable_allocator, kMaximumResourceVariables);
    }
    if (state->resource_variables == nullptr) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_TENSOR_ALLOCATION_FAILED;
        }
        return nullptr;
    }

    state->interpreter = new (std::nothrow) tflite::MicroInterpreter(
        model, state->resolver, state->tensor_arena, state->tensor_arena_size,
        state->resource_variables);
    if (state->interpreter == nullptr ||
        state->interpreter->AllocateTensors() != kTfLiteOk) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_TENSOR_ALLOCATION_FAILED;
        }
        return nullptr;
    }

    TfLiteTensor *input = state->interpreter->input(0);
    TfLiteTensor *output = state->interpreter->output(0);
    if (input == nullptr || output == nullptr || input->dims->size != 3 ||
        input->dims->data[0] != 1 || input->dims->data[1] < 1 ||
        input->dims->data[1] > UINT8_MAX ||
        input->dims->data[2] != static_cast<int>(kFeatureCount) ||
        input->type != kTfLiteInt8 || output->dims->size != 2 ||
        output->dims->data[0] != 1 || output->dims->data[1] != 1 ||
        output->type != kTfLiteUInt8) {
        echo_wakeword_destroy(state);
        if (status != nullptr) {
            *status = ECHO_WAKEWORD_MODEL_INVALID;
        }
        return nullptr;
    }

    if (status != nullptr) {
        *status = ECHO_WAKEWORD_OK;
    }
    return state;
}

void echo_wakeword_destroy(echo_wakeword_t *state) {
    if (state == nullptr) {
        return;
    }
    delete state->interpreter;
    if (state->tensor_arena != nullptr) {
        heap_caps_free(state->tensor_arena);
    }
    if (state->variable_arena != nullptr) {
        heap_caps_free(state->variable_arena);
    }
    if (state->frontend_ready) {
        FrontendFreeStateContents(&state->frontend);
    }
    delete state;
}

int32_t echo_wakeword_process(echo_wakeword_t *state,
                              const int16_t *samples,
                              size_t sample_count,
                              echo_wakeword_stats_t *stats) {
    if (state == nullptr || samples == nullptr || sample_count == 0 ||
        stats == nullptr) {
        return ECHO_WAKEWORD_INVALID_ARGUMENT;
    }

    std::memset(stats, 0, sizeof(*stats));
    stats->arena_in_psram =
        state->tensor_arena_in_psram || state->variable_arena_in_psram;
    stats->probability = state->last_probability;
    stats->average_probability = state->average_probability;
    stats->generated_feature_slices = state->generated_feature_slices;

    size_t offset = 0;
    while (offset < sample_count) {
        size_t consumed = 0;
        const FrontendOutput frontend_output = FrontendProcessSamples(
            &state->frontend, samples + offset, sample_count - offset,
            &consumed);
        if (consumed == 0) {
            return ECHO_WAKEWORD_FRONTEND_FAILED;
        }
        offset += consumed;
        if (frontend_output.size == 0) {
            continue;
        }
        if (frontend_output.values == nullptr ||
            frontend_output.size != kFeatureCount) {
            return ECHO_WAKEWORD_FRONTEND_FAILED;
        }

        ++state->generated_feature_slices;
        if (state->cooldown_remaining > 0) {
            --state->cooldown_remaining;
        }

        TfLiteTensor *input = state->interpreter->input(0);
        const uint8_t stride = static_cast<uint8_t>(input->dims->data[1]);
        state->current_stride_step %= stride;
        int8_t *input_data = tflite::GetTensorData<int8_t>(input) +
                             kFeatureCount * state->current_stride_step;
        for (size_t index = 0; index < kFeatureCount; ++index) {
            input_data[index] = quantize_feature(frontend_output.values[index]);
        }
        ++state->current_stride_step;

        if (state->current_stride_step >= stride) {
            if (state->interpreter->Invoke() != kTfLiteOk) {
                return ECHO_WAKEWORD_INFERENCE_FAILED;
            }
            stats->inference_ran = true;
            state->last_probability =
                state->interpreter->output(0)->data.uint8[0];
            state->probability_index = static_cast<uint8_t>(
                (state->probability_index + 1) % state->sliding_window_size);
            state->probabilities[state->probability_index] =
                state->last_probability;

            uint32_t sum = 0;
            for (uint8_t index = 0; index < state->sliding_window_size;
                 ++index) {
                sum += state->probabilities[index];
            }
            state->average_probability = static_cast<uint8_t>(
                sum / state->sliding_window_size);
            if (state->cooldown_remaining == 0 &&
                sum > static_cast<uint32_t>(state->probability_cutoff) *
                          state->sliding_window_size) {
                stats->detected = true;
                stats->average_probability = state->average_probability;
                std::memset(state->probabilities, 0,
                            sizeof(state->probabilities));
                state->average_probability = 0;
                state->cooldown_remaining = state->cooldown_feature_slices;
            }
        }
    }

    stats->probability = state->last_probability;
    if (!stats->detected) {
        stats->average_probability = state->average_probability;
    }
    stats->generated_feature_slices = state->generated_feature_slices;
    return ECHO_WAKEWORD_OK;
}

int32_t echo_wakeword_reset(echo_wakeword_t *state) {
    if (state == nullptr) {
        return ECHO_WAKEWORD_INVALID_ARGUMENT;
    }
    FrontendReset(&state->frontend);
    if (state->interpreter->Reset() != kTfLiteOk ||
        state->resource_variables->ResetAll() != kTfLiteOk) {
        return ECHO_WAKEWORD_INFERENCE_FAILED;
    }
    state->current_stride_step = 0;
    state->probability_index = 0;
    state->last_probability = 0;
    state->average_probability = 0;
    state->generated_feature_slices = 0;
    state->cooldown_remaining = state->cooldown_feature_slices;
    std::memset(state->probabilities, 0, sizeof(state->probabilities));
    return ECHO_WAKEWORD_OK;
}

const char *echo_wakeword_model_name(void) { return "Hey Jarvis (microWakeWord v2)"; }
