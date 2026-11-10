---
section: Changed
---
- **Breaking (Rust API):** `set_alarm_values` on a Multi-state Input or
  Value checks its list as a write does and returns `Result`, refusing a
  zero, a state past Number_Of_States or more than 1,024 entries (#2415).
