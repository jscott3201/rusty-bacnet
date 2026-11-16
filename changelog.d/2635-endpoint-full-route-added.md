---
section: Added
---
- **Rust API:** endpoint ingress counts what its full queues drop:
  `IngressDropCounters`, `EndpointIngress::full_drop_counters`,
  `DROP_LOG_INTERVAL`, and `EndpointIngress::with_coordinator`, which puts
  answers to the endpoint's own leases first (#2635).
