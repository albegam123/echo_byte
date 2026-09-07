use core::ptr::NonNull;
use std::ffi::CStr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use esp_idf_svc::sys::echo_esp_sr as ffi;

use super::{AudioFrontend, AudioMode, BackendMemory, ProcessStats, SPEAKER_CHANNELS};

pub struct SelectedAudioFrontend {
    state: NonNull<ffi::echo_esp_sr_t>,
    frame_samples: usize,
    mode: AudioMode,
}

// ESP-SR owns its internal worker state. One Rust audio task exclusively owns
// this handle and all calls require `&mut self`; it is deliberately not Sync.
unsafe impl Send for SelectedAudioFrontend {}

impl SelectedAudioFrontend {
    pub fn new() -> Result<Self> {
        let mut config = core::mem::MaybeUninit::uninit();
        unsafe {
            ffi::echo_esp_sr_default_config(config.as_mut_ptr());
        }
        let mut config = unsafe { config.assume_init() };
        config.speaker_channels = SPEAKER_CHANNELS as u32;
        config.high_performance = false;

        let mut status = ffi::ECHO_ESP_SR_OK;
        let state = unsafe { ffi::echo_esp_sr_create(&config, &mut status) };
        let state = NonNull::new(state).with_context(|| {
            format!(
                "initialize ESP-SR AFE/WakeNet (status {status}); ensure the model partition is flashed"
            )
        })?;
        let frame_samples = unsafe { ffi::echo_esp_sr_frame_samples(state.as_ptr()) } as usize;
        if frame_samples == 0 {
            unsafe { ffi::echo_esp_sr_destroy(state.as_ptr()) };
            bail!("ESP-SR reported an empty native frame");
        }
        Ok(Self {
            state,
            frame_samples,
            mode: AudioMode::Standby,
        })
    }

    fn set_mode(&mut self, mode: AudioMode) -> Result<()> {
        if self.mode == mode {
            return Ok(());
        }
        let raw_mode = match mode {
            AudioMode::Standby => ffi::echo_esp_sr_mode_t_ECHO_ESP_SR_MODE_STANDBY,
            AudioMode::FullDuplex => ffi::echo_esp_sr_mode_t_ECHO_ESP_SR_MODE_FULL_DUPLEX,
        };
        let status = unsafe { ffi::echo_esp_sr_set_mode(self.state.as_ptr(), raw_mode) };
        if status != ffi::ECHO_ESP_SR_OK {
            bail!("switch ESP-SR audio mode failed with status {status}");
        }
        self.mode = mode;
        Ok(())
    }

    fn process(
        &mut self,
        capture: &[i16],
        render: Option<&[i16]>,
        output: &mut [i16],
    ) -> Result<ProcessStats> {
        if capture.len() != self.frame_samples || output.len() != self.frame_samples {
            bail!(
                "ESP-SR capture and output frames must contain {} samples",
                self.frame_samples
            );
        }
        if render.is_some_and(|frame| frame.len() != self.frame_samples * SPEAKER_CHANNELS) {
            bail!(
                "ESP-SR render frame must contain {} stereo-interleaved samples",
                self.frame_samples * SPEAKER_CHANNELS
            );
        }

        let mut stats = core::mem::MaybeUninit::uninit();
        let status = unsafe {
            ffi::echo_esp_sr_process(
                self.state.as_ptr(),
                capture.as_ptr(),
                render.map_or(core::ptr::null(), |frame| frame.as_ptr()),
                output.as_mut_ptr(),
                self.frame_samples,
                stats.as_mut_ptr(),
            )
        };
        if status != ffi::ECHO_ESP_SR_OK {
            bail!("ESP-SR frame processing failed with status {status}");
        }
        let stats = unsafe { stats.assume_init() };
        let output_ready = stats.output_samples == self.frame_samples as u32;
        Ok(ProcessStats {
            output_ready,
            voice_active: stats.voice_active,
            // ESP-SR exposes VAD state and dBFS, but not Speex's probability
            // or current AGC gain. Keep those fields explicitly unavailable.
            speech_probability: None,
            agc_gain_db: None,
            wake_detected: stats.wake_detected,
            wake_score: None,
            wake_inference_ran: output_ready && self.mode == AudioMode::Standby,
            input_volume_dbfs: output_ready.then_some(stats.input_volume_dbfs),
            processor_time: Duration::from_micros(
                u64::from(stats.feed_time_us) + u64::from(stats.fetch_time_us),
            ),
        })
    }
}

impl AudioFrontend for SelectedAudioFrontend {
    fn backend_name() -> &'static str {
        "ESP-SR AFE + WakeNet"
    }

    fn wake_model_name(&self) -> &str {
        let pointer = unsafe { ffi::echo_esp_sr_model_name(self.state.as_ptr()) };
        if pointer.is_null() {
            return "unknown";
        }
        unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .unwrap_or("invalid model name")
    }

    fn frame_samples(&self) -> usize {
        self.frame_samples
    }

    fn memory(&self) -> BackendMemory {
        let mut memory = core::mem::MaybeUninit::uninit();
        unsafe {
            ffi::echo_esp_sr_memory(self.state.as_ptr(), memory.as_mut_ptr());
        }
        let memory = unsafe { memory.assume_init() };
        BackendMemory {
            internal_bytes: Some(memory.internal_bytes),
            psram_bytes: Some(memory.psram_bytes),
        }
    }

    fn process_standby(&mut self, capture: &[i16], output: &mut [i16]) -> Result<ProcessStats> {
        self.set_mode(AudioMode::Standby)?;
        self.process(capture, None, output)
    }

    fn process_full_duplex(
        &mut self,
        capture: &[i16],
        render: &[i16],
        output: &mut [i16],
    ) -> Result<ProcessStats> {
        self.set_mode(AudioMode::FullDuplex)?;
        self.process(capture, Some(render), output)
    }
}

impl Drop for SelectedAudioFrontend {
    fn drop(&mut self) {
        unsafe {
            ffi::echo_esp_sr_destroy(self.state.as_ptr());
        }
    }
}
