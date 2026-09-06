use core::ptr::NonNull;
use std::ffi::CStr;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use esp_idf_svc::sys::echo_wakeword as ffi;

use super::dsp::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WakeWordStats {
    pub detected: bool,
    pub inference_ran: bool,
    pub arena_in_psram: bool,
    pub probability: u8,
    pub average_probability: u8,
    pub generated_feature_slices: u32,
}

pub struct WakeWordDetector {
    state: NonNull<ffi::echo_wakeword_t>,
}

// A detector can move to its pinned audio task, while `&mut self` ensures that
// its recurrent tensors and feature frontend are never used concurrently.
unsafe impl Send for WakeWordDetector {}

impl WakeWordDetector {
    pub fn new() -> Result<Self> {
        let mut config = core::mem::MaybeUninit::uninit();
        unsafe {
            ffi::echo_wakeword_default_config(config.as_mut_ptr());
        }
        let config = unsafe { config.assume_init() };
        let mut status = ffi::ECHO_WAKEWORD_OK;
        let state = unsafe { ffi::echo_wakeword_create(&config, &mut status) };
        let state = NonNull::new(state)
            .with_context(|| format!("initialize microWakeWord detector (status {status})"))?;
        Ok(Self { state })
    }

    pub fn process(&mut self, samples: &[i16]) -> Result<WakeWordStats> {
        if samples.is_empty() {
            bail!("wake-word input cannot be empty");
        }
        let mut stats = core::mem::MaybeUninit::uninit();
        let status = unsafe {
            ffi::echo_wakeword_process(
                self.state.as_ptr(),
                samples.as_ptr(),
                samples.len(),
                stats.as_mut_ptr(),
            )
        };
        if status != ffi::ECHO_WAKEWORD_OK {
            bail!("microWakeWord streaming inference failed with status {status}");
        }
        let stats = unsafe { stats.assume_init() };
        Ok(WakeWordStats {
            detected: stats.detected,
            inference_ran: stats.inference_ran,
            arena_in_psram: stats.arena_in_psram,
            probability: stats.probability,
            average_probability: stats.average_probability,
            generated_feature_slices: stats.generated_feature_slices,
        })
    }

    pub fn reset(&mut self) -> Result<()> {
        let status = unsafe { ffi::echo_wakeword_reset(self.state.as_ptr()) };
        if status != ffi::ECHO_WAKEWORD_OK {
            bail!("reset microWakeWord detector failed with status {status}");
        }
        Ok(())
    }

    pub fn model_name() -> &'static str {
        let pointer = unsafe { ffi::echo_wakeword_model_name() };
        if pointer.is_null() {
            return "unknown";
        }
        unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .unwrap_or("invalid model name")
    }
}

impl Drop for WakeWordDetector {
    fn drop(&mut self) {
        unsafe {
            ffi::echo_wakeword_destroy(self.state.as_ptr());
        }
    }
}

pub fn run_startup_self_test() -> Result<()> {
    const TEST_FRAMES: u32 = 110;
    let initialization_started = Instant::now();
    let mut detector = WakeWordDetector::new()?;
    let initialization_elapsed = initialization_started.elapsed();
    let silence = [0_i16; FRAME_SAMPLES];
    let mut stats = None;
    let mut inference_count = 0_u32;
    let mut maximum_frame_elapsed = Duration::ZERO;
    let processing_started = Instant::now();
    for _ in 0..TEST_FRAMES {
        let frame_started = Instant::now();
        let frame_stats = detector.process(&silence)?;
        maximum_frame_elapsed = maximum_frame_elapsed.max(frame_started.elapsed());
        inference_count += u32::from(frame_stats.inference_ran);
        stats = Some(frame_stats);
    }
    let processing_elapsed = processing_started.elapsed();
    ensure!(
        inference_count > 0,
        "streaming model did not run inference during the self-test"
    );
    detector.reset()?;
    log::info!(
        "open wake-word self-test passed: model={}, {} Hz, init={:?}, {TEST_FRAMES} PCM frames={:?}, average={:?}, max={:?}, inferences={}, last={:?}",
        WakeWordDetector::model_name(),
        SAMPLE_RATE_HZ,
        initialization_elapsed,
        processing_elapsed,
        processing_elapsed / TEST_FRAMES,
        maximum_frame_elapsed,
        inference_count,
        stats.expect("self-test processes at least one frame")
    );
    Ok(())
}
