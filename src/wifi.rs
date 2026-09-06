use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use embedded_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration,
};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::ipv4::{Configuration as IpConfiguration, Mask, RouterConfiguration, Subnet};
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi, WifiDriver};
use serde::Serialize;

use crate::config::{AP_IP, AP_SSID};
use crate::provisioning::Credentials;

#[derive(Clone, Debug, Serialize)]
pub struct ScannedNetwork {
    ssid: String,
    rssi: i8,
    channel: u8,
    secure: bool,
}

pub struct WifiManager<'d> {
    wifi: BlockingWifi<EspWifi<'d>>,
}

impl<'d> WifiManager<'d> {
    pub fn new(
        modem: Modem<'d>,
        sys_loop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
    ) -> Result<Self> {
        let driver = WifiDriver::new(modem, sys_loop.clone(), Some(nvs))?;
        let sta_netif = EspNetif::new(NetifStack::Sta)?;

        let ap_netif = EspNetif::new_with_conf(&NetifConfiguration {
            ip_configuration: Some(IpConfiguration::Router(RouterConfiguration {
                subnet: Subnet {
                    gateway: AP_IP,
                    mask: Mask(24),
                },
                dhcp_enabled: true,
                // Advertise the captive DNS server itself through DHCP.
                dns: Some(AP_IP),
                secondary_dns: None,
            })),
            ..NetifConfiguration::wifi_default_router()
        })?;

        let wifi = EspWifi::wrap_all(driver, sta_netif, ap_netif)?;
        Ok(Self {
            wifi: BlockingWifi::wrap(wifi, sys_loop)?,
        })
    }

    pub fn connect_sta(&mut self, credentials: &Credentials) -> Result<()> {
        self.stop_best_effort();

        let auth_method = if credentials.password().is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2WPA3Personal
        };

        let config = Configuration::Client(ClientConfiguration {
            ssid: credentials
                .ssid()
                .try_into()
                .map_err(|_| anyhow!("SSID does not fit ESP-IDF buffer"))?,
            password: credentials
                .password()
                .try_into()
                .map_err(|_| anyhow!("password does not fit ESP-IDF buffer"))?,
            auth_method,
            ..Default::default()
        });

        self.wifi.set_configuration(&config)?;
        self.wifi.start()?;
        log::info!("connecting to SSID {:?}", credentials.ssid());

        if let Err(error) = self.wifi.connect().and_then(|_| self.wifi.wait_netif_up()) {
            self.stop_best_effort();
            return Err(error).context("STA association/DHCP failed");
        }

        let ip = self.wifi.wifi().sta_netif().get_ip_info()?;
        log::info!("state=online: DHCP info {ip:?}");
        Ok(())
    }

    pub fn reconnect(&mut self) -> Result<()> {
        if self.is_online()? {
            return Ok(());
        }
        self.wifi.connect()?;
        self.wifi.wait_netif_up()?;
        Ok(())
    }

    pub fn is_online(&self) -> Result<bool> {
        Ok(self.wifi.is_connected()? && self.wifi.is_up()?)
    }

    pub fn scan_networks(&mut self) -> Result<Vec<ScannedNetwork>> {
        self.stop_best_effort();
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
        self.wifi.start()?;

        let scan_result = self.wifi.scan();
        self.stop_best_effort();
        let mut networks: Vec<ScannedNetwork> = scan_result?
            .into_iter()
            .filter(|ap| !ap.ssid.is_empty())
            .map(|ap| ScannedNetwork {
                ssid: ap.ssid.as_str().to_owned(),
                rssi: ap.signal_strength,
                channel: ap.channel,
                secure: !matches!(ap.auth_method, None | Some(AuthMethod::None)),
            })
            .collect();

        networks.sort_by_key(|network| core::cmp::Reverse(network.rssi));
        networks.dedup_by(|a, b| a.ssid == b.ssid);
        networks.truncate(24);
        Ok(networks)
    }

    pub fn start_open_ap(&mut self) -> Result<()> {
        self.stop_best_effort();
        self.wifi
            .set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
                ssid: AP_SSID
                    .try_into()
                    .expect("AP SSID length checked at build time"),
                ssid_hidden: false,
                channel: 6,
                auth_method: AuthMethod::None,
                password: Default::default(),
                max_connections: 4,
                ..Default::default()
            }))?;
        self.wifi.start()?;
        self.wifi.wait_netif_up()?;

        let ip = self.wifi.wifi().ap_netif().get_ip_info()?;
        if ip.ip != AP_IP {
            return Err(anyhow!("AP came up at {}, expected {}", ip.ip, AP_IP));
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.wifi.is_started()? {
            self.wifi.stop()?;
        }
        Ok(())
    }

    fn stop_best_effort(&mut self) {
        if self.wifi.is_connected().unwrap_or(false) {
            let _ = self.wifi.disconnect();
        }
        if self.wifi.is_started().unwrap_or(false) {
            let _ = self.wifi.stop();
        }
        // Give the shared radio a short quiet period before changing mode.
        std::thread::sleep(Duration::from_millis(50));
    }
}
