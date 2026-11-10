---
section: Added
---
- Binary and multi-state Input, Output and Value objects built
  `with_reporting_persistence` keep the event algorithm parameters a peer
  writes (the two time delays, Alarm_Value, Alarm_Values), as the analog
  objects do, until `clear_written_reporting` (#2331). **Python API:**
  `storage_path` covers them.
