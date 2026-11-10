---
section: Migration notes
---
- **Multi-state alarm values (Rust API, #2415):** `set_alarm_values` on
  `MultiStateInputObject` and `MultiStateValueObject` returns
  `Result<(), Error>`: handle or `?` it. Set Number_Of_States before a list
  that names a state it adds; a list a write would refuse now fails here
  with the write's error instead of leaving a CONFIGURATION_ERROR.
