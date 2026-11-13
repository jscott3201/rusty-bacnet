---
section: Fixed
---
- **Wire:** a write and the event-state transition it causes now commit
  together, so a ReadProperty or COV notification no longer shows the new
  Reliability or value beside the old Event_State, such as IN_ALARM without
  FAULT just after a fault clears; that extra COV notification is gone (#2530).
