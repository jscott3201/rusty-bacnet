---
section: Changed
---
- **Breaking (Rust API):** on an Access Door or Access Zone built
  `with_reporting_persistence`, the list setters (`set_alarm_values`,
  `set_fault_values`, `set_masked_alarm_values`) no longer replace a list
  a write saved. The non-exhaustive `AlgorithmWriteConfig` also has two
  new fields (#2331).
