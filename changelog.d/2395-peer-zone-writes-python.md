---
section: Added
---
- **Python API:** `BACnetServer(clock_zone_writable=True)` lets peers write
  the Device's UTC_Offset and Daylight_Savings_Status; a write moves the
  clock as `set_clock_zone` does and lasts until `stop()` (#2395).
