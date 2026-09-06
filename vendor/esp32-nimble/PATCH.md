# Local lifecycle patch

This directory contains the source of `esp32-nimble 0.12.0` under its original
Apache-2.0 license.

`BLEDevice::deinit_full()` originally deinitialized the NimBLE host before
calling `BLEAdvertising::reset()`. The latter calls `ble_gap_adv_active()`,
which accesses host state that has already been released and produces an
ESP32-S3 `LoadProhibited` exception with ESP-IDF 5.5.3.

The local change resets advertising while the host is alive, then deinitializes
NimBLE, and finally releases Rust-side GATT services and callbacks. Remove the
`[patch.crates-io]` override after this ordering fix is available in a released
upstream crate.
