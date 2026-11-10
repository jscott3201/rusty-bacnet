---
section: Added
---
- Analog Input, Output and Value objects built `with_reporting_persistence`
  keep the event algorithm parameters a peer writes (the two time delays,
  the limits, Deadband and Limit_Enable), saved before the reply, until
  `clear_written_reporting` (#2331). **Python API:** `storage_path` covers
  them.
