---
section: Fixed
---
- A watched point no longer takes another device's values when that device
  takes its address: the point is given up, nothing more is sent for it,
  and it starts again when its own device announces itself there (#2603).
