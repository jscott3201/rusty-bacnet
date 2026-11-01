---
section: Migration notes
---
- **EnrollmentEval (Rust API, #2179):** `enrollment_eval_state_internal`,
  `set_enrollment_eval_state_internal`, `enrollment_eval_source_internal`,
  `set_enrollment_eval_source_internal` and
  `commit_event_enrollment_reliability_internal` become `eval_state`,
  `set_eval_state`, `eval_source`, `set_eval_source` and `commit_reliability`
  on `EnrollmentEval`, lent by `enrollment_eval()`/`_mut()`. Seed an Event
  Enrollment's Event_State with `set_event_state`; Alert Enrollment has no
  external seeding route.
