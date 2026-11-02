---
section: Fixed
---
- **Breaking (Rust API):** Every built-in intrinsic reporter, like Event
  Enrollment, now refuses an event transition commit that leaves NORMAL while
  Event_Detection_Enable is FALSE, with `DetectionDisabled`; the server never
  made such a commit, so nothing changes on the wire (#2271).
