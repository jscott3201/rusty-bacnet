---
section: Migration notes
---
- **Multi-state alarm values (Rust API, #2415):** `set_alarm_values` on
  `MultiStateInputObject` and `MultiStateValueObject` returns
  `Result<(), Error>`. Set it, with any count it needs, before
  `with_reporting_persistence`; after that call a restored State_Text may
  leave fewer states than the list names, so handle that refusal rather
  than end the start with `?`. A saved list wins without that check.
