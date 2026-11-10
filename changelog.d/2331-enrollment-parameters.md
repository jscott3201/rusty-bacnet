---
section: Added
---
- An Event Enrollment built `with_reporting_persistence` keeps a written
  Event_Parameters, Fault_Parameters and Time_Delay_Normal until
  `clear_written_reporting`, and is evaluated with them after a restart
  (#2331). **Python API:** `add_event_enrollment`'s `storage_path` covers
  them.
