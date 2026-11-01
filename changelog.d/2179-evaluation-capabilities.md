---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s Event Enrollment and reliability
  evaluation hooks move behind two hidden capabilities, `EnrollmentEval` and
  `ReliabilityEval`, and the unused `set_event_state_internal` and
  `set_acked_transitions_internal` are removed (#2179).
