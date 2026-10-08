---
section: Migration notes
---
- **Breaking (Rust API):** Add `renewal_interval: None` to B/IP `ForeignDeviceConfig` literals. `bip_builder().foreign_device(...)` now manages registration and renewal; `transport().bvlc_client_snapshot()` reports client BVLC outcomes and the next attempt. Cancelled requests release their slot (#938, #1013).
  Positive automatic renewal intervals below 100 ms use a 100 ms local pacing floor, including busy-slot retries and local send failures. Advertised TTL values are unchanged.
