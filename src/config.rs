use core::net::Ipv4Addr;

pub const AP_SSID: &str = "Activate_CyberToy";
pub const BLE_NAME: &str = "EchoByte-Setup";
pub const AP_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 4, 1);
pub const PORTAL_URL: &str = "http://192.168.4.1/";

pub const BLE_SERVICE_UUID: &str = "7b3e0001-6d6f-4d65-9f20-6563686f6279";
pub const BLE_CREDENTIALS_UUID: &str = "7b3e0002-6d6f-4d65-9f20-6563686f6279";
pub const BLE_INFO_UUID: &str = "7b3e0003-6d6f-4d65-9f20-6563686f6279";
pub const BLE_NETWORKS_UUID: &str = "7b3e0004-6d6f-4d65-9f20-6563686f6279";

pub const MAX_CREDENTIAL_JSON_LEN: usize = 256;
pub const MAX_SAVED_WIFI_PROFILES: usize = 8;
pub const MAX_WIFI_PROFILES_BLOB_LEN: usize = 8 * 1024;
pub const STA_CONNECT_ATTEMPTS: usize = 3;
pub const ONLINE_POLL_SECS: u64 = 2;
pub const ONLINE_RECONNECT_ATTEMPTS: usize = 3;
