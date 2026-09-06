mod ble;
mod dns;
mod portal;

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::wifi::{ScannedNetwork, WifiManager};

pub use ble::BleProvisioner;
pub use portal::CaptivePortal;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Credentials {
    ssid: String,
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct WireCredentials {
    pub ssid: String,
    #[serde(default)]
    pub password: String,
}

impl Credentials {
    pub fn new(ssid: &str, password: &str) -> core::result::Result<Self, &'static str> {
        if ssid.is_empty() {
            return Err("SSID cannot be empty");
        }
        if ssid.len() > 32 {
            return Err("SSID is longer than 32 bytes");
        }
        if ssid.contains('\0') {
            return Err("SSID contains a NUL byte");
        }
        if password.len() > 64 {
            return Err("Password is longer than 64 bytes");
        }
        if !password.is_empty() && password.len() < 8 {
            return Err("Password must be empty or at least 8 bytes");
        }
        if password.contains('\0') {
            return Err("Password contains a NUL byte");
        }

        Ok(Self {
            ssid: ssid.to_owned(),
            password: password.to_owned(),
        })
    }

    pub(crate) fn from_json(data: &[u8]) -> core::result::Result<Self, &'static str> {
        if data.len() > crate::config::MAX_CREDENTIAL_JSON_LEN {
            return Err("Credential payload is too large");
        }

        let wire: WireCredentials =
            serde_json::from_slice(data).map_err(|_| "Expected JSON with ssid and password")?;
        Self::new(&wire.ssid, &wire.password)
    }

    pub fn ssid(&self) -> &str {
        &self.ssid
    }

    pub fn password(&self) -> &str {
        &self.password
    }
}

/// Runs AP portal and BLE GATT provisioning at the same time. Both producers
/// race on a capacity-one channel; only the first valid submission is accepted.
pub async fn obtain_credentials(wifi: &mut WifiManager<'_>) -> Result<Credentials> {
    log::info!("state=provisioning: starting AP and BLE tracks");

    let networks = wifi.scan_networks().unwrap_or_else(|error| {
        log::warn!("Wi-Fi scan failed; manual SSID entry remains available: {error:#}");
        Vec::new()
    });
    wifi.start_open_ap().context("start provisioning AP")?;

    let (sender, receiver) = async_channel::bounded::<Credentials>(1);
    let winner_chosen = Arc::new(AtomicBool::new(false));

    // Resource destruction order matters: HTTP/DNS and BLE are dropped before
    // the coordinator switches the Wi-Fi driver back to STA mode.
    let portal = CaptivePortal::start(sender.clone(), winner_chosen.clone(), &networks)
        .context("start captive portal services")?;
    let ble = BleProvisioner::start(sender, winner_chosen, &networks)
        .context("start BLE provisioning")?;

    log::info!(
        "provisioning ready: AP={} portal={} BLE={}",
        crate::config::AP_SSID,
        crate::config::PORTAL_URL,
        crate::config::BLE_NAME
    );

    let credentials = wait_for_winner(&receiver).await?;
    log::info!(
        "credentials received for SSID {:?}; stopping provisioning",
        credentials.ssid()
    );
    // Let HTTP finish its response or NimBLE finish its ATT acknowledgement
    // before destroying the winning transport.
    std::thread::sleep(Duration::from_millis(200));

    drop(ble);
    drop(portal);
    wifi.stop().context("stop provisioning AP")?;

    Ok(credentials)
}

async fn wait_for_winner(receiver: &Receiver<Credentials>) -> Result<Credentials> {
    receiver
        .recv()
        .await
        .context("all provisioning producers stopped")
}

pub(crate) fn submit(
    sender: &Sender<Credentials>,
    winner_chosen: &AtomicBool,
    data: &[u8],
) -> core::result::Result<(), &'static str> {
    let credentials = Credentials::from_json(data)?;
    winner_chosen
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another provisioning request already won")?;

    if sender.try_send(credentials).is_err() {
        winner_chosen.store(false, Ordering::Release);
        return Err("Provisioning coordinator is unavailable");
    }
    Ok(())
}

pub(crate) fn networks_json(networks: &[ScannedNetwork]) -> Result<String> {
    serde_json::to_string(networks).context("serialize scanned Wi-Fi list")
}
