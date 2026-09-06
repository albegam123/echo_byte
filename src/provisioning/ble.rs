use anyhow::{Context, Result};
use async_channel::Sender;
use esp32_nimble::{uuid128, BLEAdvertisementData, BLEDevice, NimbleProperties};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::{submit, Credentials};
use crate::config::{BLE_CREDENTIALS_UUID, BLE_INFO_UUID, BLE_NAME, BLE_SERVICE_UUID};

pub struct BleProvisioner;

impl BleProvisioner {
    pub fn start(sender: Sender<Credentials>, winner_chosen: Arc<AtomicBool>) -> Result<Self> {
        // `take()` initializes the lazy singleton only once; explicit init is
        // required when a failed STA attempt restarts provisioning later.
        BLEDevice::init();
        let device = BLEDevice::take();
        BLEDevice::set_device_name(BLE_NAME).context("set BLE GAP device name")?;
        device
            .set_preferred_mtu(256)
            .context("set BLE preferred MTU")?;

        let advertising = device.get_advertising();
        let server = device.get_server();
        server.advertise_on_disconnect(true);
        server.on_connect(|server, desc| {
            log::info!("BLE provisioning client connected");
            if let Err(error) = server.update_conn_params(desc.conn_handle(), 12, 24, 0, 200) {
                log::warn!("BLE connection parameter update failed: {error:?}");
            }
        });
        server.on_disconnect(|_, reason| {
            log::info!("BLE provisioning client disconnected: {reason:?}");
        });

        let service = server.create_service(uuid128!("7b3e0001-6d6f-4d65-9f20-6563686f6279"));

        let info = service.lock().create_characteristic(
            uuid128!("7b3e0003-6d6f-4d65-9f20-6563686f6279"),
            NimbleProperties::READ,
        );
        info.lock()
            .set_value(br#"{"version":1,"format":{"ssid":"...","password":"..."}}"#);

        let credentials = service.lock().create_characteristic(
            uuid128!("7b3e0002-6d6f-4d65-9f20-6563686f6279"),
            NimbleProperties::WRITE | NimbleProperties::WRITE_NO_RSP,
        );
        credentials.lock().on_write(move |args| {
            match submit(&sender, &winner_chosen, args.recv_data()) {
                Ok(()) => log::info!("BLE provisioning request accepted"),
                Err(error) => {
                    log::warn!("BLE provisioning request rejected: {error}");
                    args.reject();
                }
            }
        });

        advertising.lock().set_data(
            BLEAdvertisementData::new()
                .name(BLE_NAME)
                .add_service_uuid(uuid128!("7b3e0001-6d6f-4d65-9f20-6563686f6279")),
        )?;
        advertising.lock().start()?;

        log::info!(
            "BLE GATT UUIDs: service={BLE_SERVICE_UUID} credentials={BLE_CREDENTIALS_UUID} info={BLE_INFO_UUID}"
        );
        Ok(Self)
    }
}

impl Drop for BleProvisioner {
    fn drop(&mut self) {
        if let Err(error) = BLEDevice::deinit_full() {
            log::warn!("failed to deinitialize NimBLE: {error}");
        } else {
            log::info!("BLE provisioning stopped and memory released");
        }
    }
}
