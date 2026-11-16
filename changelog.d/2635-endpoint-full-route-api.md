---
section: Changed
---
- **Breaking (Rust API):** `SessionConfig::queue_capacity` defaults to 256,
  `PolicyCountersSnapshot` gains four `*_full_drops` counts and is
  non-exhaustive, and `ClassifierExit::PolicyRouteFull`,
  `PolicyReason::RouteFull` and `SessionExit::Ingress` are removed (#2635).
