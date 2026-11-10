---
section: Changed
---
- **Breaking (Rust API):** `AlgorithmWriteConfig` gains `alarm_value` and
  `alarm_values`, two more parameters of kind 6 of the `RBNWCF01` format,
  and `set_alarm_values` on a Multi-state Input or Value gives way to a
  list a write saved (#2331).
