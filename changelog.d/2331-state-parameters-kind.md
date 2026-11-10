---
section: Changed
---
- **Breaking (Rust API):** `set_alarm_values` on a Multi-state Input or
  Value built `with_reporting_persistence` gives way to an Alarm_Values
  list a write saved. `AlgorithmWriteConfig` gains `alarm_value` and
  `alarm_values`, two more parameters of kind 6 of `RBNWCF01` (#2331).
