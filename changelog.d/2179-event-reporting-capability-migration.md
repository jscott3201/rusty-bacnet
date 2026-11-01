---
section: Migration notes
---
- **EventReporting (Rust API, #2179):** move the intrinsic-reporting hooks
  into `impl EventReporting`, lent by `event_reporting()`/`_mut()`; calls on a
  concrete type need `use bacnet_objects::traits::EventReporting`. Commit,
  inhibit and enrollment-summary hooks drop `_internal`; the correlated hooks
  become `acknowledge_alarm(state, &timestamp)`, and the bits-only
  `acknowledge_alarm` is removed ([details](docs/rust-api.md#capabilities)).
