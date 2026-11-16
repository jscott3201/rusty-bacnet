---
section: Changed
---
- **Breaking (Rust API):** `BACnetClient::cov_notifications()` yields a
  `CovEvent`, `Object` or `Multiple`, so a SubscribeCOVPropertyMultiple
  subscription's notifications arrive on it in order with the others; the
  ack policy and `Station::is_source_of` take a `&CovEvent` (#2654).
