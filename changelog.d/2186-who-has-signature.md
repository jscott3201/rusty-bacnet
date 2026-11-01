---
section: Changed
---
- **Breaking (Rust API):** `bacnet-device-core`'s `discovery::who_has` takes
  the `DeviceIdentity`, as `who_is` does, and picks the Device itself (#2186).
