---
section: Added
---
- **Python API:** `add_event_enrollment` takes `event_parameters`,
  `fault_parameters` and `time_delay_normal` keywords. Like the other
  builders' parameter keywords, they apply before storage attaches, so a
  peer's saved write still goes over them and they are never saved (#2426).
