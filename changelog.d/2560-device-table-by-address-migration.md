---
section: Migration notes
---
- **Discovery table by address (Rust API, #2560):** handle the `Result` from
  `get_device` and `Engine::device` (`Error::AmbiguousDevice` lists each
  claimant); call `DeviceTable::find` for `get(instance)` and
  `get(network, mac)`, network 0 local, for `get_by_mac` and
  `get_by_network_address`; read `instance` and `claimants` from
  `DeviceCollisionEvent`; give each device its own address, as one MAC
  holds one. Python: catch `BacnetAmbiguousDeviceError`.
