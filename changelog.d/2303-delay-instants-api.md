---
section: Changed
---
- **Breaking (Rust API):** `EventReporting`'s intrinsic evaluation and tick,
  and the detectors' `evaluate`, `probe` and `tick`, take a
  `bacnet_time::Instant`; `next_due` says when a pending transition is due;
  `bacnet-device-core`'s `event::after_write`, `sweep` and `next_due` take
  instants (#2303).
