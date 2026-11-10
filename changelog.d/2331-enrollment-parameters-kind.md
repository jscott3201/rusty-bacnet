---
section: Changed
---
- **Breaking (Rust API):** on an Event Enrollment built
  `with_reporting_persistence`, `set_event_parameters`,
  `set_fault_parameters` and `set_time_delay_normal` no longer replace a
  value a write saved. The non-exhaustive `AlgorithmWriteConfig` also has
  two new fields (#2331).
