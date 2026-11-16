---
section: Migration notes
---
- **Endpoint full queues (Rust API, #2635):** drop matches on
  `ClassifierExit::PolicyRouteFull`, `PolicyReason::RouteFull` and
  `SessionExit::Ingress` (never produced): a full queue no longer ends the
  session or reaches policy; read its drops from `policy_counters()` or
  `EndpointIngress::full_drop_counters()`. Build a `PolicyCountersSnapshot`
  with `Default` instead of a literal. Set `queue_capacity` to 16 to keep
  the old size.
