---
section: Changed
---
- **Breaking (Rust API):** `bacnet-device-core` has the
  SubscribeCOVPropertyMultiple service, the capture of timestamped changes
  and the split of a report to the subscriber's APDU size, behind
  `cov-multiple`. `cov::timed`'s sizing helpers are private again.
  `bacnet-server` runs on them (#2320).
