---
section: Changed
---
- **Breaking (Rust API):** `bacnet-device-core` gains an `event-reporting`
  feature, on with `std`, whose `event` module holds intrinsic reporting's
  commit and sweep, distribution, AcknowledgeAlarm, GetEventInformation and
  the fault pass, moved from `bacnet-server`, which runs on them (#2303).
