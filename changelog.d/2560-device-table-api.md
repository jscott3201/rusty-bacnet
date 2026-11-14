---
section: Changed
---
- **Breaking (Rust API):** `DeviceTable::get(instance)` becomes
  `find(instance)`, `get(network, mac)` replaces `get_by_mac` and
  `get_by_network_address`, and `claiming(instance)` and
  `DiscoveredDevice::address()` are new; `upsert` leaves other addresses'
  rows for the instance (#2560).
