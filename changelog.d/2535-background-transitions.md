---
section: Fixed
---
- **Wire:** Fault detection, Schedule passes, membership relinquishes and
  the operation task commit the event transition with their change: its
  notification goes out then, not a sweep later, and their COV reports
  carry FAULT and IN_ALARM together, as does the core's fault pass (#2535).
