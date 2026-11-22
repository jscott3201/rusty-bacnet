---
section: Fixed
---
- With duplicate suppression on, a router still learns from a repeated
  I-Am-Router-To-Network where the route stays on its port, so a station
  that announced a router's networks first no longer keeps that router's own
  announcement out. The repeat is still not passed on (#2713).
