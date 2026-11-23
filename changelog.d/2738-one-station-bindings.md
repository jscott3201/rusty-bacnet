---
section: Added
---
- **Rust API:** `DeviceBinding::one_station()` and an endpoint session's
  `with_source_audit_routed_one_station_binding` mark a routed binding one
  station, so the server's confirmed notifications, writes and audit records
  reach a device at such an address (#2738).
