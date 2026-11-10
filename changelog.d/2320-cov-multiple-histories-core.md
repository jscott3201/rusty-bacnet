---
section: Changed
---
- **Rust API:** `bacnet-device-core` has the SubscribeCOVPropertyMultiple
  notification path's timestamped histories (`cov::timed`) and read cache
  (`cov::multiple_reads`), behind a `cov-multiple` feature that `std` turns
  on. `bacnet-server` runs on them; nothing changes on the wire (#2320).
