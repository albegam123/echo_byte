use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition, EspNvs};

use crate::provisioning::Credentials;

const NAMESPACE: &str = "echo_cfg";
const KEY_SSID: &str = "wifi_ssid";
const KEY_PASSWORD: &str = "wifi_pass";
const KEY_VALID: &str = "wifi_valid";

pub struct CredentialStore {
    nvs: EspDefaultNvs,
}

impl CredentialStore {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        Ok(Self {
            nvs: EspNvs::new(partition, NAMESPACE, true).context("open echo_byte NVS namespace")?,
        })
    }

    pub fn load(&self) -> Result<Option<Credentials>> {
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

        Ok(Credentials::new(ssid, password).ok())
    }

    pub fn save(&self, credentials: &Credentials) -> Result<()> {
        // The marker is written last so a reset between writes cannot expose a
        // half-updated credential pair as valid.
        let _ = self.nvs.remove(KEY_VALID)?;
        self.nvs.set_str(KEY_SSID, credentials.ssid())?;
        self.nvs.set_str(KEY_PASSWORD, credentials.password())?;
        self.nvs.set_u8(KEY_VALID, 1)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn clear(&self) -> Result<()> {
        let _ = self.nvs.remove(KEY_VALID)?;
        let _ = self.nvs.remove(KEY_SSID)?;
        let _ = self.nvs.remove(KEY_PASSWORD)?;
        Ok(())
    }
}
