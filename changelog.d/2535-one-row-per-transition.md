---
section: Changed
---
- **Wire:** A timestamped COV-multiple subscription no longer gets a row
  showing an object between a write and the event-state transition it
  causes: that is folded into the newest row the write, or an earlier one
  to the object in the request, queued, on the server and the core's
  `Reporter` (#2535).
