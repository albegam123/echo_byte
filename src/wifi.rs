use std::collections::HashSet;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use embedded_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration,
};
use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::ipv4::{Configuration as IpConfiguration, Mask, RouterConfiguration, Subnet};
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi, WifiDriver, WifiEvent};
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

impl ScannedNetwork {
    pub fn ssid(&self) -> &str {
        &self.ssid
    }

    pub fn rssi(&self) -> i8 {
        self.rssi
    }
}

pub struct WifiManager<'d> {
    wifi: BlockingWifi<EspWifi<'d>>,
    // Keep this subscription alive for the lifetime of the driver so failed
    // WPA handshakes report an actionable ESP-IDF reason instead of a generic
    // 15-second connection timeout.
    _event_logger: EspSubscription<'static, System>,
}

impl<'d> WifiManager<'d> {
    pub fn new(
        modem: Modem<'d>,
        sys_loop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
    ) -> Result<Self> {
        let event_logger = sys_loop.subscribe::<WifiEvent, _>(|event| match event {
            WifiEvent::StaConnected(details) => {
                log::info!("STA associated: {details:?}");
            }
            WifiEvent::StaDisconnected(details) => {
                log::warn!("STA disconnected: {details:?}");
            }
            _ => {}
        })?;

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
            _event_logger: event_logger,
        })
    }

    pub fn connect_sta(&mut self, credentials: &Credentials) -> Result<()> {
        self.stop_best_effort();

        let auth_method = if credentials.password().is_empty() {
            AuthMethod::None
        } else {
            // ESP-IDF treats this field as the minimum accepted auth mode,
            // not as an exact mode selection. WPA2 therefore accepts both
            // ordinary WPA2-PSK and stronger WPA3/mixed-mode access points.
            AuthMethod::WPA2Personal
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
        self.disable_power_save()?;
        log::info!("connecting to SSID {:?}", credentials.ssid());

        let connection_result = self.wifi.connect().and_then(|_| {
            // The AP -> STA transition can leave lwIP's DHCP client in a
            // stopped/old state on ESP-IDF 5.5. Restart it explicitly after
            // association so a fresh DISCOVER is sent for every candidate.
            self.wait_sta_netif_ready()?;
            self.restart_sta_dhcp()?;
            self.wifi.wait_netif_up()
        });

        if let Err(error) = connection_result {
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
            .set_configuration(&Configuration::Client(ClientConfiguration {
                // Scanning does not authenticate. Explicitly allowing open
                // networks also avoids the driver's misleading empty-password
                // warning while this temporary STA configuration is active.
                auth_method: AuthMethod::None,
                ..Default::default()
            }))?;
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
        // APs using the same SSID are not necessarily adjacent after an RSSI
        // sort. Retain the strongest one while preserving signal order.
        let mut seen = HashSet::new();
        networks.retain(|network| seen.insert(network.ssid.clone()));
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

    fn restart_sta_dhcp(&self) -> core::result::Result<(), esp_idf_svc::sys::EspError> {
        use esp_idf_svc::sys::{
            esp_netif_dhcp_status_t, esp_netif_dhcp_status_t_ESP_NETIF_DHCP_STARTED,
            esp_netif_dhcpc_get_status, esp_netif_dhcpc_start, esp_netif_dhcpc_stop, EspError,
            ESP_ERR_ESP_NETIF_DHCP_ALREADY_STARTED, ESP_ERR_ESP_NETIF_DHCP_ALREADY_STOPPED, ESP_OK,
        };

        let handle = self.wifi.wifi().sta_netif().handle();
        let mut status: esp_netif_dhcp_status_t = 0;
        EspError::check_and_return(
            unsafe { esp_netif_dhcpc_get_status(handle, &mut status) },
            (),
        )?;
        log::info!("STA DHCP client status before restart: {status}");

        let stop_result = unsafe { esp_netif_dhcpc_stop(handle) };
        if !matches!(stop_result, ESP_OK | ESP_ERR_ESP_NETIF_DHCP_ALREADY_STOPPED) {
            return Err(EspError::from(stop_result).expect("non-zero DHCP stop error"));
        }

        let start_result = unsafe { esp_netif_dhcpc_start(handle) };
        if !matches!(
            start_result,
            ESP_OK | ESP_ERR_ESP_NETIF_DHCP_ALREADY_STARTED
        ) {
            return Err(EspError::from(start_result).expect("non-zero DHCP start error"));
        }

        let mut restarted_status: esp_netif_dhcp_status_t = 0;
        EspError::check_and_return(
            unsafe { esp_netif_dhcpc_get_status(handle, &mut restarted_status) },
            (),
        )?;
        if restarted_status != esp_netif_dhcp_status_t_ESP_NETIF_DHCP_STARTED {
            return Err(EspError::from_infallible::<
                { esp_idf_svc::sys::ESP_ERR_INVALID_STATE },
            >());
        }
        log::info!("STA DHCP client restarted");
        Ok(())
    }

    fn disable_power_save(&self) -> core::result::Result<(), esp_idf_svc::sys::EspError> {
        use esp_idf_svc::sys::{esp_wifi_set_ps, wifi_ps_type_t_WIFI_PS_NONE, EspError};

        EspError::check_and_return(unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) }, ())?;
        log::info!("STA modem-sleep disabled for reliable low-latency traffic");
        Ok(())
    }

    fn wait_sta_netif_ready(&self) -> core::result::Result<(), esp_idf_svc::sys::EspError> {
        for _ in 0..100 {
            if self.wifi.wifi().sta_netif().is_netif_up()? {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        Err(esp_idf_svc::sys::EspError::from_infallible::<
            { esp_idf_svc::sys::ESP_ERR_INVALID_STATE },
        >())
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
