use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use esp_idf_svc::hal::gpio::{Input, PinDriver};

const EVENT_QUEUE_DEPTH: usize = 4;
const BUTTON_SAMPLE_MS: u64 = 10;
const BUTTON_DEBOUNCE_SAMPLES: u8 = 3;

pub type WakeEventSender = Sender<WakeEvent>;
pub type WakeEventReceiver = Receiver<WakeEvent>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakeSource {
    BootButton,
    Voice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WakeEvent {
    pub source: WakeSource,
    pub monotonic_us: u64,
}

impl WakeEvent {
    pub fn new(source: WakeSource) -> Self {
        let monotonic_us = unsafe { esp_idf_svc::sys::esp_timer_get_time() }.max(0) as u64;
        Self {
            source,
            monotonic_us,
        }
    }
}

pub fn event_channel() -> (WakeEventSender, WakeEventReceiver) {
    async_channel::bounded(EVENT_QUEUE_DEPTH)
}

/// Publish a voice-model detection into the same queue used by the physical
/// button. A full queue means the consumer is already behind, so duplicate
/// wake requests are deliberately coalesced instead of blocking the audio path.
#[allow(dead_code)]
pub fn publish_voice(sender: &WakeEventSender) {
    let _ = sender.try_send(WakeEvent::new(WakeSource::Voice));
}

pub struct ButtonWakeTask {
    _thread: JoinHandle<()>,
}

impl ButtonWakeTask {
    pub fn start(button: PinDriver<'static, Input>, sender: WakeEventSender) -> Result<Self> {
        let thread = std::thread::Builder::new()
            .name("wake-button".into())
            .stack_size(4096)
            .spawn(move || run_button(button, sender))
            .context("start BOOT-button wake task")?;
        Ok(Self { _thread: thread })
    }
}

fn run_button(button: PinDriver<'static, Input>, sender: WakeEventSender) {
    let mut pressed_samples = 0_u8;
    let mut released_samples = BUTTON_DEBOUNCE_SAMPLES;
    let mut latched = false;

    loop {
        if button.is_low() {
            pressed_samples = pressed_samples
                .saturating_add(1)
                .min(BUTTON_DEBOUNCE_SAMPLES);
            released_samples = 0;
            if pressed_samples == BUTTON_DEBOUNCE_SAMPLES && !latched {
                latched = true;
                let _ = sender.try_send(WakeEvent::new(WakeSource::BootButton));
            }
        } else {
            released_samples = released_samples
                .saturating_add(1)
                .min(BUTTON_DEBOUNCE_SAMPLES);
            pressed_samples = 0;
            if released_samples == BUTTON_DEBOUNCE_SAMPLES {
                latched = false;
            }
        }
        std::thread::sleep(Duration::from_millis(BUTTON_SAMPLE_MS));
    }
}
