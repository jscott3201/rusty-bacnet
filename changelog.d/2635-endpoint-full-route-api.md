---
section: Changed
---
- **Breaking (Rust API):** `SessionConfig::queue_capacity` defaults to 256,
  `PolicyCountersSnapshot` gains four `*_full_drops` counts and is
  non-exhaustive, and `ClassifierExit::PolicyRouteFull` and
  `PolicyReason::RouteFull` are removed (#2635).
