---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s event-reporting hooks move to an
  `EventReporting` capability lent through `event_reporting()` and
  `event_reporting_mut()`. One `acknowledge_alarm` replaces the plain,
  correlated and detailed acknowledgment hooks (#2179).
