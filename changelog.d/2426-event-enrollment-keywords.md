---
section: Added
---
- **Python API:** `add_event_enrollment` takes `object_property_reference`,
  `event_parameters`, `fault_parameters` and `time_delay_normal` keywords.
  The parameter keywords apply before storage attaches, so a peer's saved
  write still goes over them and they are never saved (#2426).
