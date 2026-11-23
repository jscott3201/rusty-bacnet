---
section: Migration notes
---
- **Routed targets (Rust API, #2738):** a pattern on `Target::RoutedMac { dnet, mac }` adds `one_station` or `..`. Send to a `DeviceAddress` through `Station::from(address)`, not `BACnetAddress`, which drops the mark.
