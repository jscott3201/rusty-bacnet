---
section: Fixed
---
- **Breaking (Rust API):** Event Enrollment refuses an event transition commit
  that leaves NORMAL while Event_Detection_Enable is FALSE, with the new
  `EventTransitionCommitError::DetectionDisabled` (#2179).
