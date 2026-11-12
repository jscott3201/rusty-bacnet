---
section: Changed
---
- **Breaking (Rust API):** `EventReporting`'s intrinsic evaluation and tick
  take a `bacnet_time::Instant`, `next_due` says when a pending transition is
  due, and `PendingTransition` holds that instant; `bacnet-device-core`'s
  `event::after_write` and `event::sweep` take it too (#2303).
