use core::ptr::NonNull;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use esp_idf_svc::sys::echo_audio_dsp as ffi;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const FRAME_SAMPLES: usize = 160;

#[derive(Clone, Copy, Debug)]
pub struct AudioDspConfig {
    pub echo_tail_ms: u32,
    pub speaker_channels: usize,
    pub noise_suppression_db: i32,
    pub echo_suppression_db: i32,
    pub echo_suppression_active_db: i32,
    pub agc_target: i32,
    pub agc_max_gain_db: i32,
    pub vad_start_probability: i32,
    pub vad_continue_probability: i32,
}

impl Default for AudioDspConfig {
    fn default() -> Self {
        Self {
            echo_tail_ms: 100,
            speaker_channels: 2,
            noise_suppression_db: -20,
            echo_suppression_db: -40,
            echo_suppression_active_db: -15,
            agc_target: 12_000,
            agc_max_gain_db: 20,
            vad_start_probability: 80,
            vad_continue_probability: 65,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioDspStats {
    pub voice_active: bool,
    pub speech_probability: i32,
    pub agc_gain_db: i32,
}

pub struct AudioDsp {
    state: NonNull<ffi::echo_audio_dsp_t>,
    speaker_channels: usize,
}

// SpeexDSP state is independent and may be moved to its pinned audio task,
// but all mutation remains serialized through `&mut self`. It is not `Sync`.
unsafe impl Send for AudioDsp {}

impl AudioDsp {
    pub fn new(config: AudioDspConfig) -> Result<Self> {
        let mut raw_config = core::mem::MaybeUninit::uninit();
        unsafe {
            ffi::echo_audio_dsp_default_config(raw_config.as_mut_ptr());
        }
        let mut raw_config = unsafe { raw_config.assume_init() };
        raw_config.echo_tail_samples = config
            .echo_tail_ms
            .checked_mul(SAMPLE_RATE_HZ)
            .context("echo tail length overflow")?
            / 1_000;
        raw_config.speaker_channels = config
            .speaker_channels
            .try_into()
            .context("speaker channel count does not fit the C ABI")?;
        raw_config.noise_suppression_db = config.noise_suppression_db;
        raw_config.echo_suppression_db = config.echo_suppression_db;
        raw_config.echo_suppression_active_db = config.echo_suppression_active_db;
        raw_config.agc_target = config.agc_target;
        raw_config.agc_max_gain_db = config.agc_max_gain_db;
        raw_config.vad_start_probability = config.vad_start_probability;
        raw_config.vad_continue_probability = config.vad_continue_probability;

        let mut status = ffi::ECHO_AUDIO_DSP_OK;
        let state = unsafe { ffi::echo_audio_dsp_create(&raw_config, &mut status) };
        let state = NonNull::new(state)
            .with_context(|| format!("initialize SpeexDSP AEC/NS/AGC/VAD (status {status})"))?;
        Ok(Self {
            state,
            speaker_channels: config.speaker_channels,
        })
    }

    pub fn process(
        &mut self,
        capture: &[i16],
        render: Option<&[i16]>,
        output: &mut [i16],
    ) -> Result<AudioDspStats> {
        if capture.len() != FRAME_SAMPLES || output.len() != FRAME_SAMPLES {
            bail!("capture and output frames must contain {FRAME_SAMPLES} samples");
        }
        let render_samples = FRAME_SAMPLES * self.speaker_channels;
        if render.is_some_and(|frame| frame.len() != render_samples) {
            bail!("interleaved render reference frame must contain {render_samples} samples");
        }

        let mut stats = core::mem::MaybeUninit::uninit();
        let status = unsafe {
            ffi::echo_audio_dsp_process(
                self.state.as_ptr(),
                capture.as_ptr(),
                render.map_or(core::ptr::null(), |frame| frame.as_ptr()),
                output.as_mut_ptr(),
                FRAME_SAMPLES,
                stats.as_mut_ptr(),
            )
        };
        if status != ffi::ECHO_AUDIO_DSP_OK {
            bail!("SpeexDSP frame processing failed with status {status}");
        }
        let stats = unsafe { stats.assume_init() };
        Ok(AudioDspStats {
            voice_active: stats.voice_active,
            speech_probability: stats.speech_probability,
            agc_gain_db: stats.agc_gain_db,
        })
    }

    pub fn reset_aec(&mut self) {
        unsafe {
            ffi::echo_audio_dsp_reset_aec(self.state.as_ptr());
        }
    }
}

impl Drop for AudioDsp {
    fn drop(&mut self) {
        unsafe {
            ffi::echo_audio_dsp_destroy(self.state.as_ptr());
        }
    }
}

pub fn run_startup_self_test() -> Result<()> {
    const TEST_FRAMES: u32 = 100;
    let initialization_started = Instant::now();
    let mut dsp = AudioDsp::new(AudioDspConfig::default())?;
    let initialization_elapsed = initialization_started.elapsed();
    let capture = [0_i16; FRAME_SAMPLES];
    let render = [0_i16; FRAME_SAMPLES * 2];
    let mut output = [0_i16; FRAME_SAMPLES];
    let mut stats = None;
    let mut maximum_frame_elapsed = Duration::ZERO;
    let processing_started = Instant::now();
    for _ in 0..TEST_FRAMES {
        let frame_started = Instant::now();
        stats = Some(dsp.process(&capture, Some(&render), &mut output)?);
        maximum_frame_elapsed = maximum_frame_elapsed.max(frame_started.elapsed());
    }
    let processing_elapsed = processing_started.elapsed();
    dsp.reset_aec();
    log::info!(
        "open SpeexDSP 3A self-test passed: init={:?}, {TEST_FRAMES} frames={:?}, average={:?}, max={:?}, last={:?}",
        initialization_elapsed,
        processing_elapsed,
        processing_elapsed / TEST_FRAMES,
        maximum_frame_elapsed,
        stats.expect("self-test processes at least one frame")
    );
    Ok(())
}
