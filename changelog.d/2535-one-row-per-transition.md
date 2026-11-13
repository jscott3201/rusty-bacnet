---
section: Changed
---
- **Wire:** A timestamped COV-multiple subscription gets one row for a write
  and the event-state transition it causes, from the hosted server and the
  core's `Reporter` alike, instead of a row before the transition and one
  after it (#2535).
