---
section: Changed
---
- **Breaking (Rust API):** `BACnetClient::get_device` and `Engine::device`
  return `Result<Option<DiscoveredDevice>, Error>`, not `Option`; the new
  `BACnetClient::get_device_at(network, mac)` finds a device by address, and
  `discovered_devices()` lists one row per address (#2560).
