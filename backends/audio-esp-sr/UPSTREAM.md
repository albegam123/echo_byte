# ESP-SR source provenance

The `audio-esp-sr` backend uses the Espressif Component Registry package:

- Component: `espressif/esp-sr`
- Version: `2.5.3`
- Upstream commit: `efa8d907c6d457cd0f99dae6c6b493412d3078d4`
- Registry component SHA-256: `c3dd4d3a3ce520c90e6a48370f41ff1e138cea128929270ea03ee74e6710cea3`
- License: ESPRESSIF MIT License (use without charge is restricted to
  Espressif Systems products)

The dependency and its precompiled algorithm archives are downloaded at build
time and are not vendored in this repository. `components_esp32s3.lock` pins
the resolved package and transitive component versions. Redistributed firmware
must include the notice in `LICENSE-ESPRESSIF-MIT.txt`.
