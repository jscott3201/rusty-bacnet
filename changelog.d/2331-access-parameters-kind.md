---
section: Changed
---
- **Breaking (Rust API):** on an Access Door or Access Zone built
  `with_reporting_persistence`, `set_alarm_values`, `set_fault_values` and
  `set_masked_alarm_values` give way to a list a write saved.
  `AlgorithmWriteConfig` gains `fault_values` and `masked_alarm_values`
  (#2331).
