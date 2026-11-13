---
section: Changed
---
- **Wire:** A timestamped COV-multiple subscription no longer gets a row
  showing an object between a write and the event-state transition it
  causes: the transition is folded into the newest row the write queued,
  from the hosted server and the core's `Reporter` alike (#2535).
