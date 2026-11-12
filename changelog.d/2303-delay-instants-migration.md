---
section: Migration notes
---
- **Intrinsic delays (Rust API, #2303):** pass your monotonic `now` (a
  `bacnet_time::Instant`) to `evaluate_intrinsic_reporting`,
  `tick_intrinsic_reporting`, the detectors and `event::after_write` and
  `event::sweep`, and sweep again at `event::next_due`. A custom
  `EventReporting` that counted ticks compares `now` with its own due
  instant instead and returns it from `next_due`: the tick is no longer
  once a second. `PendingTransition::remaining` is now `due`.
