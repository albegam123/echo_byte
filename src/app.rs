use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use crate::config::{ONLINE_POLL_SECS, ONLINE_RECONNECT_ATTEMPTS};
use crate::provisioning::{self, Credentials};
use crate::storage::CredentialStore;
use crate::wifi::WifiManager;

pub async fn run() -> Result<()> {
    let peripherals = Peripherals::take().context("take ESP32-S3 peripherals")?;
    let sys_loop = EspSystemEventLoop::take().context("take ESP system event loop")?;
    let nvs = EspDefaultNvsPartition::take().context("initialize default NVS partition")?;

    let store = CredentialStore::new(nvs.clone())?;
    let mut wifi = WifiManager::new(peripherals.modem, sys_loop, nvs)?;

    let mut candidate = store.load()?;
    if let Some(credentials) = candidate.as_ref() {
        log::info!(
            "stored Wi-Fi credentials found for SSID {:?}",
            credentials.ssid()
        );
    } else {
        log::info!("no stored Wi-Fi credentials; entering provisioning");
    }

    loop {
        let credentials = match candidate.take() {
            Some(credentials) => credentials,
            None => provisioning::obtain_credentials(&mut wifi).await?,
        };

        match wifi.connect_sta(&credentials) {
            Ok(()) => {
                store.save(&credentials)?;
                log::info!("credentials committed to NVS after successful DHCP");

                if monitor_connection(&mut wifi, &credentials).await? {
                    candidate = Some(credentials);
                }
            }
            Err(error) => {
                // Never persist an unverified submission. Both provisioning
                // tracks are restarted so the user can correct the password.
                log::warn!(
                    "could not connect to SSID {:?}: {error:#}; restarting provisioning",
                    credentials.ssid()
                );
            }
        }
    }
}

/// Returns true when reconnect attempts were exhausted and the caller should
/// retry the stored credentials once before falling back to provisioning.
async fn monitor_connection(wifi: &mut WifiManager<'_>, credentials: &Credentials) -> Result<bool> {
    loop {
        std::thread::sleep(Duration::from_secs(ONLINE_POLL_SECS));
        if wifi.is_online().unwrap_or(false) {
            continue;
        }

        log::warn!("Wi-Fi link lost; attempting automatic reconnect");
        for attempt in 1..=ONLINE_RECONNECT_ATTEMPTS {
            match wifi.reconnect() {
                Ok(()) => {
                    log::info!("Wi-Fi reconnected on attempt {attempt}");
                    break;
                }
                Err(error) if attempt == ONLINE_RECONNECT_ATTEMPTS => {
                    log::warn!(
                        "reconnect failed for SSID {:?}: {error:#}",
                        credentials.ssid()
                    );
                    return Ok(true);
                }
                Err(error) => {
                    log::warn!("reconnect attempt {attempt} failed: {error:#}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}
