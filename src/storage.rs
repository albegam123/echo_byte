use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition, EspNvs};
use serde::{Deserialize, Serialize};

use crate::config::{MAX_SAVED_WIFI_PROFILES, MAX_WIFI_PROFILES_BLOB_LEN};
use crate::provisioning::Credentials;

const NAMESPACE: &str = "echo_cfg";
const KEY_PROFILES: &str = "wifi_profiles";
const PROFILES_VERSION: u8 = 1;

// Legacy single-profile keys. They remain readable so deployed boards migrate
// without asking the user for the same password again.
const KEY_SSID: &str = "wifi_ssid";
const KEY_PASSWORD: &str = "wifi_pass";
const KEY_VALID: &str = "wifi_valid";

#[derive(Deserialize, Serialize)]
struct StoredProfiles {
    version: u8,
    entries: Vec<StoredCredential>,
}

#[derive(Deserialize, Serialize)]
struct StoredCredential {
    ssid: String,
    password: String,
}

pub struct CredentialStore {
    nvs: EspDefaultNvs,
}

impl CredentialStore {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        Ok(Self {
            nvs: EspNvs::new(partition, NAMESPACE, true).context("open echo_byte NVS namespace")?,
        })
    }

    /// Loads profiles in most-recently-successful order. A malformed or
    /// unsupported blob is discarded safely instead of preventing the device
    /// from returning to provisioning mode.
    pub fn load_all(&self) -> Result<Vec<Credentials>> {
        match self.read_profiles_blob() {
            Ok(Some(profiles)) => Ok(profiles),
            Ok(None) => self.migrate_legacy_profile(),
            Err(error) => {
                log::warn!("stored Wi-Fi profile list is invalid; ignoring it: {error:#}");
                let _ = self.nvs.remove(KEY_PROFILES);
                self.migrate_legacy_profile()
            }
        }
    }

    /// Inserts or updates a successfully connected profile and moves it to the
    /// front of the MRU list. The least recently successful entry is evicted
    /// when the configured capacity is reached.
    pub fn record_success(&self, credentials: &Credentials) -> Result<Vec<Credentials>> {
        let mut profiles = self.load_all()?;
        profiles.retain(|stored| stored.ssid() != credentials.ssid());
        profiles.insert(0, credentials.clone());
        profiles.truncate(MAX_SAVED_WIFI_PROFILES);
        self.write_profiles(&profiles)?;
        Ok(profiles)
    }

    #[allow(dead_code)]
    pub fn clear_all(&self) -> Result<()> {
        let _ = self.nvs.remove(KEY_PROFILES)?;
        self.remove_legacy_profile()?;
        Ok(())
    }

    fn read_profiles_blob(&self) -> Result<Option<Vec<Credentials>>> {
        let Some(blob_len) = self.nvs.blob_len(KEY_PROFILES)? else {
            return Ok(None);
        };
        if blob_len == 0 || blob_len > MAX_WIFI_PROFILES_BLOB_LEN {
            bail!("profile blob length {blob_len} is outside the accepted range");
        }

        let mut blob = vec![0_u8; blob_len];
        let Some(data) = self.nvs.get_blob(KEY_PROFILES, &mut blob)? else {
            return Ok(None);
        };
        let stored: StoredProfiles =
            serde_json::from_slice(data).context("decode Wi-Fi profile blob")?;
        if stored.version != PROFILES_VERSION {
            bail!(
                "unsupported Wi-Fi profile version {} (expected {PROFILES_VERSION})",
                stored.version
            );
        }

        let mut seen = HashSet::new();
        let mut profiles = Vec::with_capacity(stored.entries.len().min(MAX_SAVED_WIFI_PROFILES));
        for entry in stored.entries {
            if profiles.len() == MAX_SAVED_WIFI_PROFILES {
                break;
            }
            match Credentials::new(&entry.ssid, &entry.password) {
                Ok(credentials) if seen.insert(entry.ssid) => profiles.push(credentials),
                Ok(_) => log::warn!("duplicate stored Wi-Fi profile skipped"),
                Err(error) => log::warn!("invalid stored Wi-Fi profile skipped: {error}"),
            }
        }
        Ok(Some(profiles))
    }

    fn write_profiles(&self, profiles: &[Credentials]) -> Result<()> {
        let stored = StoredProfiles {
            version: PROFILES_VERSION,
            entries: profiles
                .iter()
                .take(MAX_SAVED_WIFI_PROFILES)
                .map(|credentials| StoredCredential {
                    ssid: credentials.ssid().to_owned(),
                    password: credentials.password().to_owned(),
                })
                .collect(),
        };
        let blob = serde_json::to_vec(&stored).context("encode Wi-Fi profile blob")?;
        if blob.len() > MAX_WIFI_PROFILES_BLOB_LEN {
            bail!(
                "encoded Wi-Fi profile blob is {} bytes (limit {})",
                blob.len(),
                MAX_WIFI_PROFILES_BLOB_LEN
            );
        }
        self.nvs
            .set_blob(KEY_PROFILES, &blob)
            .context("write Wi-Fi profile blob")?;
        Ok(())
    }

    fn migrate_legacy_profile(&self) -> Result<Vec<Credentials>> {
        let Some(credentials) = self.read_legacy_profile()? else {
            return Ok(Vec::new());
        };

        let profiles = vec![credentials];
        self.write_profiles(&profiles)?;
        self.remove_legacy_profile()?;
        log::info!("migrated legacy Wi-Fi credential to the profile list");
        Ok(profiles)
    }

    fn read_legacy_profile(&self) -> Result<Option<Credentials>> {
        if self.nvs.get_u8(KEY_VALID)? != Some(1) {
            return Ok(None);
        }

        let mut ssid_buf = [0_u8; 33];
        let mut password_buf = [0_u8; 65];
        let Some(ssid) = self.nvs.get_str(KEY_SSID, &mut ssid_buf)? else {
            return Ok(None);
        };
        let Some(password) = self.nvs.get_str(KEY_PASSWORD, &mut password_buf)? else {
            return Ok(None);
        };

        match Credentials::new(ssid, password) {
            Ok(credentials) => Ok(Some(credentials)),
            Err(error) => {
                log::warn!("legacy Wi-Fi credential is invalid; ignoring it: {error}");
                Ok(None)
            }
        }
    }

    fn remove_legacy_profile(&self) -> Result<()> {
        // The validity marker is removed first so an interrupted cleanup can
        // never make a partial legacy pair authoritative again.
        let _ = self.nvs.remove(KEY_VALID)?;
        let _ = self.nvs.remove(KEY_SSID)?;
        let _ = self.nvs.remove(KEY_PASSWORD)?;
        Ok(())
    }
}
