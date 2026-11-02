---
section: Migration notes
---
- **Log hooks (Rust API, #2179):** `log_buffer_internal`,
  `log_record_identities_internal`, `add_trend_record`,
  `add_trend_multiple_record` and `add_event_log_record` leave `BACnetObject`.
  To add a record to a built-in Trend Log, Trend Log Multiple or Event Log,
  call its own `add_record`. A custom log lends the hidden `LogBuffer` from
  `log_buffer()` and `log_buffer_mut()`.
