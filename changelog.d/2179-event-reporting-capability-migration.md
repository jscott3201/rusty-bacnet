---
section: Migration notes
---
- **EventReporting (Rust API, #2179):** move `evaluate_intrinsic_reporting`,
  `tick_intrinsic_reporting` and the hooks below into `impl EventReporting`,
  lent by `event_reporting()` and `event_reporting_mut()`.
  `commit_event_transition_internal` is `commit_event_transition`; the
  inhibit pair drops `_internal`. The acknowledgment hooks become
  `acknowledge_alarm(state, &timestamp)`, returning `Option<EventStateChange>`
  ([before and after](docs/rust-api.md#capabilities)).
