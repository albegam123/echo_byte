use std::time::Instant;

use anyhow::{bail, Result};
use esp_idf_svc::sys::{
    heap_caps_get_free_size, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM,
};

use super::dsp::{AudioDsp, AudioDspConfig, FRAME_SAMPLES};
use super::wakeword::WakeWordDetector;
use super::{AudioFrontend, AudioMode, BackendMemory, ProcessStats, SPEAKER_CHANNELS};

pub struct SelectedAudioFrontend {
    dsp: AudioDsp,
    wakeword: WakeWordDetector,
    mode: AudioMode,
    memory: BackendMemory,
}

impl SelectedAudioFrontend {
    pub fn new() -> Result<Self> {
        let internal_before =
            unsafe { heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) };
        let psram_before = unsafe { heap_caps_get_free_size(MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT) };
        let dsp = AudioDsp::new(AudioDspConfig::default())?;
        let wakeword = WakeWordDetector::new()?;
        let internal_after =
            unsafe { heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) };
        let psram_after = unsafe { heap_caps_get_free_size(MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT) };
        Ok(Self {
            dsp,
            wakeword,
            mode: AudioMode::Standby,
            memory: BackendMemory {
                internal_bytes: Some(internal_before.saturating_sub(internal_after) as u32),
                psram_bytes: Some(psram_before.saturating_sub(psram_after) as u32),
            },
        })
    }

    fn validate_frames(&self, capture: &[i16], output: &[i16]) -> Result<()> {
        if capture.len() != FRAME_SAMPLES || output.len() != FRAME_SAMPLES {
            bail!("open backend frames must contain {FRAME_SAMPLES} samples");
        }
        Ok(())
    }
}

impl AudioFrontend for SelectedAudioFrontend {
    fn backend_name() -> &'static str {
        "SpeexDSP + microWakeWord"
    }

    fn wake_model_name(&self) -> &str {
        WakeWordDetector::model_name()
    }

    fn frame_samples(&self) -> usize {
        FRAME_SAMPLES
    }

    fn memory(&self) -> BackendMemory {
        self.memory
    }

    fn process_standby(&mut self, capture: &[i16], output: &mut [i16]) -> Result<ProcessStats> {
        self.validate_frames(capture, output)?;
        if self.mode != AudioMode::Standby {
            self.dsp.reset_aec();
            self.wakeword.reset()?;
            self.mode = AudioMode::Standby;
        }

        let started = Instant::now();
        let dsp = self.dsp.process(capture, None, output)?;
        let wake = self.wakeword.process(output)?;
        Ok(ProcessStats {
            output_ready: true,
            voice_active: dsp.voice_active,
            speech_probability: Some(dsp.speech_probability),
            agc_gain_db: Some(dsp.agc_gain_db),
            wake_detected: wake.detected,
            wake_score: Some(wake.average_probability),
            wake_inference_ran: wake.inference_ran,
            input_volume_dbfs: None,
            processor_time: started.elapsed(),
        })
    }

    fn process_full_duplex(
        &mut self,
        capture: &[i16],
        render: &[i16],
        output: &mut [i16],
    ) -> Result<ProcessStats> {
        self.validate_frames(capture, output)?;
        if render.len() != FRAME_SAMPLES * SPEAKER_CHANNELS {
            bail!(
                "open backend render frame must contain {} stereo-interleaved samples",
                FRAME_SAMPLES * SPEAKER_CHANNELS
            );
        }
        self.mode = AudioMode::FullDuplex;

        let started = Instant::now();
        let dsp = self.dsp.process(capture, Some(render), output)?;
        Ok(ProcessStats {
            output_ready: true,
            voice_active: dsp.voice_active,
            speech_probability: Some(dsp.speech_probability),
            agc_gain_db: Some(dsp.agc_gain_db),
            wake_detected: false,
            wake_score: None,
            wake_inference_ran: false,
            input_volume_dbfs: None,
            processor_time: started.elapsed(),
        })
    }
}
