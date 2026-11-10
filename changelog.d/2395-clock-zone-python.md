---
section: Added
---
- **Python API:** `BACnetServer(utc_offset_minutes=..., daylight_savings=...)`
  starts the Device clock in a zone, and `set_clock_zone(utc_offset_minutes,
  daylight_savings)` changes it while the server runs; the clock keeps its
  UTC instant (#2395).
