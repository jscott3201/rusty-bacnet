---
section: Changed
---
- **Wire:** A confirmed event notification that finds every invoke ID in use
  waits for one instead of being dropped. Only its destination waits: its
  lane, or for a Device recipient waiting for its I-Am that device's queue,
  which now holds 64 as a lane does (#2537).
