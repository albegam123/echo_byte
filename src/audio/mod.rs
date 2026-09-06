pub mod dsp;
pub mod wakeword;

use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub fn run_startup_self_test() -> Result<()> {
    dsp::run_startup_self_test().context("SpeexDSP self-test")?;
    wakeword::run_startup_self_test().context("microWakeWord self-test")?;
    run_standby_pipeline_self_test().context("standby wake pipeline self-test")?;
    Ok(())
}

fn run_standby_pipeline_self_test() -> Result<()> {
    const TEST_FRAMES: u32 = 110;
    const FRAME_DEADLINE: Duration = Duration::from_millis(10);

    let mut dsp = dsp::AudioDsp::new(dsp::AudioDspConfig::default())?;
    let mut detector = wakeword::WakeWordDetector::new()?;
    let capture = [0_i16; dsp::FRAME_SAMPLES];
    let mut processed = [0_i16; dsp::FRAME_SAMPLES];
    let mut maximum_frame_elapsed = Duration::ZERO;
    let mut deadline_misses = 0_u32;
    let mut inference_count = 0_u32;
    let started = Instant::now();

    for _ in 0..TEST_FRAMES {
        let frame_started = Instant::now();
        // The speaker is silent while guarding for the wake word, so AEC is
        // intentionally bypassed. AEC and wake inference must not be run in
        // every frame at the same time on one core.
        dsp.process(&capture, None, &mut processed)?;
        let wake_stats = detector.process(&processed)?;
        inference_count += u32::from(wake_stats.inference_ran);
        let frame_elapsed = frame_started.elapsed();
        maximum_frame_elapsed = maximum_frame_elapsed.max(frame_elapsed);
        deadline_misses += u32::from(frame_elapsed > FRAME_DEADLINE);
    }
    let elapsed = started.elapsed();

    log::info!(
        "standby open wake pipeline self-test passed: {TEST_FRAMES} frames={:?}, average={:?}, max={:?}, deadline_misses={}, inferences={}",
        elapsed,
        elapsed / TEST_FRAMES,
        maximum_frame_elapsed,
        deadline_misses,
        inference_count
    );
    Ok(())
}
