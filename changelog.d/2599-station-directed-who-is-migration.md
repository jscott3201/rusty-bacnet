---
section: Migration notes
---
- **Breaking (Rust API, #2599):** `bacnet_ops::Scope::Directed` and `Discover::directed` take a `bacnet_ops::Station` (the low-level `Station`, re-exported); convert a `DeviceAddress` with `.into()`. `OpsError::UnknownDevice.asked` is an `Option<Scope>`: for a remote address it is the station's network, asked after a Who-Is addressed to the station drew nothing.
