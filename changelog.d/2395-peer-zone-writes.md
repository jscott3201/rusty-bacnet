---
section: Added
---
- **Breaking (Rust API):** Peers may write the Device's UTC_Offset and
  Daylight_Savings_Status when the host opts in (`clock_zone_writable` on the
  server builders, `DeviceClock::with_writable_zone` in the device core); a
  Device with Tags storage keeps each written half across a restart (#2395).
