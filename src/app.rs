use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use crate::config::{ONLINE_POLL_SECS, ONLINE_RECONNECT_ATTEMPTS, STA_CONNECT_ATTEMPTS};
use crate::provisioning::{self, Credentials};
use crate::storage::CredentialStore;
use crate::wifi::{ScannedNetwork, WifiManager};

pub async fn run() -> Result<()> {
    let peripherals = Peripherals::take().context("take ESP32-S3 peripherals")?;
    let sys_loop = EspSystemEventLoop::take().context("take ESP system event loop")?;
    let nvs = EspDefaultNvsPartition::take().context("initialize default NVS partition")?;

    let store = CredentialStore::new(nvs.clone())?;
    let mut wifi = WifiManager::new(peripherals.modem, sys_loop, nvs)?;
    let mut profiles = store.load_all()?;

    if profiles.is_empty() {
        log::info!("no stored Wi-Fi profiles; entering provisioning");
    } else {
        log::info!("loaded {} stored Wi-Fi profile(s)", profiles.len());
    }

    loop {
        if let Some(credentials) = connect_known_profiles(&mut wifi, &profiles) {
            profiles = store.record_success(&credentials)?;
            log::info!(
                "Wi-Fi profile {:?} moved to the front of the MRU list",
                credentials.ssid()
            );
            monitor_connection(&mut wifi, &credentials).await?;
            continue;
        }

        // Stay in provisioning until a newly submitted credential completes
        // both association and DHCP. Failed submissions are never persisted.
        loop {
            let credentials = provisioning::obtain_credentials(&mut wifi).await?;
            match connect_with_retries(&mut wifi, &credentials) {
                Ok(()) => {
                    profiles = store.record_success(&credentials)?;
                    log::info!(
                        "Wi-Fi profile {:?} committed after successful DHCP ({} saved)",
                        credentials.ssid(),
                        profiles.len()
                    );
                    monitor_connection(&mut wifi, &credentials).await?;
                    break;
                }
                Err(error) => {
                    // Both provisioning tracks are restarted so the user can
                    // correct the SSID or password.
                    log::warn!(
                        "could not connect to SSID {:?}: {error:#}; restarting provisioning",
                        credentials.ssid()
                    );
                }
            }
        }
    }
}

/// Scans first so profiles that cannot exist at the current location do not
/// each consume a full association timeout. Visible profiles are tried once in
/// descending signal order, with MRU position as the stable tie-breaker.
fn connect_known_profiles(
    wifi: &mut WifiManager<'_>,
    profiles: &[Credentials],
) -> Option<Credentials> {
    if profiles.is_empty() {
        return None;
    }

    let candidates = match wifi.scan_networks() {
        Ok(networks) => {
            let candidates = rank_visible_profiles(profiles, &networks);
            if candidates.is_empty() {
                log::info!(
                    "none of the {} stored Wi-Fi profiles is visible",
                    profiles.len()
                );
            } else {
                log::info!(
                    "{} of {} stored Wi-Fi profile(s) visible; trying strongest first",
                    candidates.len(),
                    profiles.len()
                );
            }
            candidates
        }
        Err(error) => {
            log::warn!(
                "Wi-Fi scan failed; falling back to all stored profiles in MRU order: {error:#}"
            );
            profiles
                .iter()
                .cloned()
                .map(|credentials| (credentials, None))
                .collect()
        }
    };

    let candidate_count = candidates.len();
    for (position, (credentials, rssi)) in candidates.into_iter().enumerate() {
        match rssi {
            Some(rssi) => log::info!(
                "trying known Wi-Fi {}/{}: SSID {:?}, RSSI {} dBm",
                position + 1,
                candidate_count,
                credentials.ssid(),
                rssi
            ),
            None => log::info!(
                "trying known Wi-Fi {}/{}: SSID {:?}",
                position + 1,
                candidate_count,
                credentials.ssid()
            ),
        }

        match wifi.connect_sta(&credentials) {
            Ok(()) => return Some(credentials),
            Err(error) => log::warn!(
                "known Wi-Fi SSID {:?} failed: {error:#}; trying next profile",
                credentials.ssid()
            ),
        }
    }

    None
}

fn rank_visible_profiles(
    profiles: &[Credentials],
    networks: &[ScannedNetwork],
) -> Vec<(Credentials, Option<i8>)> {
    let mut candidates: Vec<(usize, Credentials, i8)> = profiles
        .iter()
        .enumerate()
        .filter_map(|(mru_position, credentials)| {
            networks
                .iter()
                .find(|network| network.ssid() == credentials.ssid())
                .map(|network| (mru_position, credentials.clone(), network.rssi()))
        })
        .collect();

    candidates.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    candidates
        .into_iter()
        .map(|(_, credentials, rssi)| (credentials, Some(rssi)))
        .collect()
}

fn connect_with_retries(wifi: &mut WifiManager<'_>, credentials: &Credentials) -> Result<()> {
    let mut last_error = None;

    for attempt in 1..=STA_CONNECT_ATTEMPTS {
        match wifi.connect_sta(credentials) {
            Ok(()) => return Ok(()),
            Err(error) => {
                log::warn!(
                    "STA attempt {attempt}/{STA_CONNECT_ATTEMPTS} for SSID {:?} failed: {error:#}",
                    credentials.ssid()
                );
                last_error = Some(error);
                if attempt < STA_CONNECT_ATTEMPTS {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    Err(last_error.expect("STA_CONNECT_ATTEMPTS is non-zero"))
}

/// Monitors the active profile until its direct reconnect attempts are
/// exhausted. The caller then rescans all known profiles for location failover.
async fn monitor_connection(wifi: &mut WifiManager<'_>, credentials: &Credentials) -> Result<()> {
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
                        "reconnect failed for SSID {:?}: {error:#}; rescanning all profiles",
                        credentials.ssid()
                    );
                    return Ok(());
                }
                Err(error) => {
                    log::warn!("reconnect attempt {attempt} failed: {error:#}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}
