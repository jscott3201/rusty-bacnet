---
section: Migration notes
---
- **Written configuration (Rust API, #2331):** `AlgorithmWriteConfig` gains
  `alarm_value` and `alarm_values`; a custom `WriteConfigPersistence` that
  stores the fields itself must keep them. An older release refuses a binary
  or multi-state object's file holding a parameter: run
  `clear_written_reporting` before a downgrade. Saved Alarm_Values naming a
  state past the rebuilt object's Number_Of_States fail
  `with_reporting_persistence`.
