---
section: Fixed
---
- **Wire:** A confirmed COV notification held back by its subscriber's
  in-flight limit or the server's pool of 255 now waits, as one refused an
  invoke ID does, and goes with the current value once there is room;
  `notifications_throttled_peer` counts each report held back (#2671).
