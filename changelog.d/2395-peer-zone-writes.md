---
section: Added
---
- **Breaking (Rust API):** Peers may write the Device's UTC_Offset and
  Daylight_Savings_Status when the host opts in (`clock_zone_writable`;
  `zone-writes` in the device core). A Device with Tags storage keeps each
  half; the newest change, a peer's or the host's, wins (#2395).
