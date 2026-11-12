---
section: Migration notes
---
- **Intrinsic delays (Rust API, #2303):** pass your monotonic `now`
  (`bacnet_objects::event::Instant`, re-exported) to the intrinsic
  evaluation and tick, the detectors and `event::after_write`/`sweep`, and
  sweep again at `event::next_due(db, now)`. A custom `EventReporting` keeps
  its own due instant and returns it from `next_due`; a wrapper must forward
  `next_due`, or its delays end at whole-second sweeps.
