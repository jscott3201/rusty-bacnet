---
section: Migration notes
---
- **Breaking (Rust API).** Pass a `bacnet_client::client::Station` to `bacnet_ops::Scope::Directed` and `Discover::directed`. Convert an existing `DeviceAddress` with `.into()`. Match `OpsError::UnknownDevice.asked` against `Option<Scope>` to distinguish a station-directed lookup from a broadcast. (#2599).
