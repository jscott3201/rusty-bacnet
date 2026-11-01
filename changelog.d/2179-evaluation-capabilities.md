---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s reliability hooks move to a public
  `ReliabilityEval` capability lent through `reliability_eval()`, the Event
  Enrollment evaluator's hooks to a hidden `EnrollmentEval`, and the unused
  `set_event_state_internal` and `set_acked_transitions_internal` go (#2179).
