#[cfg(all(feature = "audio-open", not(feature = "audio-esp-sr")))]
pub mod dsp;
#[cfg(all(feature = "audio-open", not(feature = "audio-esp-sr")))]
mod open;
#[cfg(all(feature = "audio-open", not(feature = "audio-esp-sr")))]
pub mod wakeword;

#[cfg(all(feature = "audio-esp-sr", not(feature = "audio-open")))]
mod esp_sr;

use std::time::Duration;

#[cfg(any(
    all(feature = "audio-open", not(feature = "audio-esp-sr")),
    all(feature = "audio-esp-sr", not(feature = "audio-open"))
))]
use std::time::Instant;

use anyhow::Result;

#[cfg(all(feature = "audio-open", not(feature = "audio-esp-sr")))]
pub use open::SelectedAudioFrontend;

#[cfg(all(feature = "audio-esp-sr", not(feature = "audio-open")))]
pub use esp_sr::SelectedAudioFrontend;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
#[allow(dead_code)]
pub const SPEAKER_CHANNELS: usize = 2;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioMode {
    Standby,
    FullDuplex,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessStats {
    /// False while a pipelined backend is warming up and has no output yet.
    pub output_ready: bool,
    pub voice_active: bool,
    pub speech_probability: Option<i32>,
    pub agc_gain_db: Option<i32>,
    pub wake_detected: bool,
    pub wake_score: Option<u8>,
    pub wake_inference_ran: bool,
    pub input_volume_dbfs: Option<f32>,
    pub processor_time: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendMemory {
    pub internal_bytes: Option<u32>,
    pub psram_bytes: Option<u32>,
}

/// Compile-time-selected audio pipeline contract.
///
/// Input and output are mono signed-16 PCM at 16 kHz. The full-duplex render
/// frame is stereo-interleaved. Frame length is queried because SpeexDSP uses
/// 10 ms while ESP-SR FD AFE natively uses 32 ms.
#[allow(dead_code)]
pub trait AudioFrontend {
    fn backend_name() -> &'static str;
    fn wake_model_name(&self) -> &str;
    fn frame_samples(&self) -> usize;
    fn memory(&self) -> BackendMemory;

    fn process_standby(&mut self, capture: &[i16], output: &mut [i16]) -> Result<ProcessStats>;

    fn process_full_duplex(
        &mut self,
        capture: &[i16],
        render: &[i16],
        output: &mut [i16],
    ) -> Result<ProcessStats>;
}

#[cfg(all(feature = "audio-open", not(feature = "audio-esp-sr")))]
pub fn run_startup_self_test() -> Result<()> {
    use anyhow::Context;

    dsp::run_startup_self_test().context("SpeexDSP self-test")?;
    wakeword::run_startup_self_test().context("microWakeWord self-test")?;
    run_selected_pipeline_self_test().context("selected open pipeline self-test")?;
    Ok(())
}

#[cfg(all(feature = "audio-esp-sr", not(feature = "audio-open")))]
pub fn run_startup_self_test() -> Result<()> {
    run_selected_pipeline_self_test()
}

#[cfg(not(any(
    all(feature = "audio-open", not(feature = "audio-esp-sr")),
    all(feature = "audio-esp-sr", not(feature = "audio-open"))
)))]
pub fn run_startup_self_test() -> Result<()> {
    unreachable!("audio backend feature validation failed")
}

#[cfg(any(
    all(feature = "audio-open", not(feature = "audio-esp-sr")),
    all(feature = "audio-esp-sr", not(feature = "audio-open"))
))]
fn run_selected_pipeline_self_test() -> Result<()> {
    const STANDBY_TEST_FRAMES: u32 = 110;
    const FULL_DUPLEX_TEST_FRAMES: u32 = 100;
    const PIPELINE_WARMUP_FRAMES: u32 = 8;
    let initialization_started = Instant::now();
    let mut frontend = SelectedAudioFrontend::new()?;
    let initialization_elapsed = initialization_started.elapsed();
    let frame_samples = frontend.frame_samples();
    let frame_deadline = Duration::from_secs_f64(frame_samples as f64 / SAMPLE_RATE_HZ as f64);
    let capture = vec![0_i16; frame_samples];
    let mut processed = vec![0_i16; frame_samples];
    let mut warmup_ready = false;
    for _ in 0..PIPELINE_WARMUP_FRAMES {
        let frame_started = Instant::now();
        warmup_ready |= frontend
            .process_standby(&capture, &mut processed)?
            .output_ready;
        let frame_elapsed = frame_started.elapsed();
        if frame_elapsed < frame_deadline {
            std::thread::sleep(frame_deadline - frame_elapsed);
        }
    }
    anyhow::ensure!(warmup_ready, "standby backend did not warm up");

    let mut maximum_processor_time = Duration::ZERO;
    let mut processor_time = Duration::ZERO;
    let mut deadline_misses = 0_u32;
    let mut inference_count = 0_u32;
    let mut output_count = 0_u32;
    let mut last_stats = None;
    let started = Instant::now();

    for _ in 0..STANDBY_TEST_FRAMES {
        let frame_started = Instant::now();
        let stats = frontend.process_standby(&capture, &mut processed)?;
        inference_count += u32::from(stats.wake_inference_ran);
        output_count += u32::from(stats.output_ready);
        processor_time += stats.processor_time;
        maximum_processor_time = maximum_processor_time.max(stats.processor_time);
        last_stats = Some(stats);
        let frame_elapsed = frame_started.elapsed();
        deadline_misses += u32::from(frame_elapsed > frame_deadline);
        if frame_elapsed < frame_deadline {
            std::thread::sleep(frame_deadline - frame_elapsed);
        }
    }
    let elapsed = started.elapsed();

    log::info!(
        "audio standby self-test passed: backend={}, model={}, frame={} samples ({:?}), init={:?}, memory={:?}, {} frames wall={:?}, adapter_average={:?}, adapter_max={:?}, deadline_misses={}, outputs={}, wake_frames={}, last={:?}",
        SelectedAudioFrontend::backend_name(),
        frontend.wake_model_name(),
        frame_samples,
        frame_deadline,
        initialization_elapsed,
        frontend.memory(),
        STANDBY_TEST_FRAMES,
        elapsed,
        processor_time / STANDBY_TEST_FRAMES,
        maximum_processor_time,
        deadline_misses,
        output_count,
        inference_count,
        last_stats.expect("self-test processes at least one frame")
    );
    anyhow::ensure!(
        output_count * 100 >= STANDBY_TEST_FRAMES * 95,
        "standby backend produced only {output_count}/{STANDBY_TEST_FRAMES} output frames"
    );
    anyhow::ensure!(
        deadline_misses == 0,
        "standby backend missed {deadline_misses} frame deadlines"
    );

    let render = vec![0_i16; frame_samples * SPEAKER_CHANNELS];
    let mut warmup_ready = false;
    for _ in 0..PIPELINE_WARMUP_FRAMES {
        let frame_started = Instant::now();
        warmup_ready |= frontend
            .process_full_duplex(&capture, &render, &mut processed)?
            .output_ready;
        let frame_elapsed = frame_started.elapsed();
        if frame_elapsed < frame_deadline {
            std::thread::sleep(frame_deadline - frame_elapsed);
        }
    }
    anyhow::ensure!(warmup_ready, "full-duplex backend did not warm up");

    let mut maximum_processor_time = Duration::ZERO;
    let mut processor_time = Duration::ZERO;
    let mut deadline_misses = 0_u32;
    let mut output_count = 0_u32;
    let mut last_stats = None;
    let started = Instant::now();
    for _ in 0..FULL_DUPLEX_TEST_FRAMES {
        let frame_started = Instant::now();
        let stats = frontend.process_full_duplex(&capture, &render, &mut processed)?;
        processor_time += stats.processor_time;
        maximum_processor_time = maximum_processor_time.max(stats.processor_time);
        output_count += u32::from(stats.output_ready);
        last_stats = Some(stats);
        let frame_elapsed = frame_started.elapsed();
        deadline_misses += u32::from(frame_elapsed > frame_deadline);
        if frame_elapsed < frame_deadline {
            std::thread::sleep(frame_deadline - frame_elapsed);
        }
    }
    let elapsed = started.elapsed();
    log::info!(
        "audio full-duplex self-test passed: backend={}, frame={} samples ({:?}), {} frames wall={:?}, adapter_average={:?}, adapter_max={:?}, deadline_misses={}, outputs={}, last={:?}",
        SelectedAudioFrontend::backend_name(),
        frame_samples,
        frame_deadline,
        FULL_DUPLEX_TEST_FRAMES,
        elapsed,
        processor_time / FULL_DUPLEX_TEST_FRAMES,
        maximum_processor_time,
        deadline_misses,
        output_count,
        last_stats.expect("self-test processes at least one frame")
    );
    anyhow::ensure!(
        output_count * 100 >= FULL_DUPLEX_TEST_FRAMES * 95,
        "full-duplex backend produced only {output_count}/{FULL_DUPLEX_TEST_FRAMES} output frames"
    );
    anyhow::ensure!(
        deadline_misses == 0,
        "full-duplex backend missed {deadline_misses} frame deadlines"
    );
    Ok(())
}
