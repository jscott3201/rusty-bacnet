---
section: Migration notes
---
- **Breaking (Rust API):** Add `renewal_interval: None` to B/IP `ForeignDeviceConfig` literals. `bip_builder().foreign_device(...)` now manages registration and renewal; `transport().bvlc_client_snapshot()` reports client BVLC outcomes and the next attempt. Cancelled requests release their slot (#938, #1013).
