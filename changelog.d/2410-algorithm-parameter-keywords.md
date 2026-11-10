---
section: Added
---
- **Python API:** the analog, binary and multi-state builders take keywords
  for the event algorithm parameters they keep, such as `high_limit`,
  `alarm_value` and `time_delay`. They apply before storage attaches, so a
  peer's saved write still goes over them and they are never saved (#2410).
