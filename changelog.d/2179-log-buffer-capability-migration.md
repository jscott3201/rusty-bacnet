---
section: Migration notes
---
- **Log hooks (Rust API, #2179):** `log_buffer_internal`,
  `log_record_identities_internal` and the three `add_*_record` hooks leave
  `BACnetObject`. Add a record to a built-in Trend Log, Trend Log Multiple or
  Event Log with its own `add_record`; a custom log lends the hidden
  `LogBuffer` from `log_buffer()` and `log_buffer_mut()`. A wrapper around a
  built-in COV Trend Log or Event Log no longer reaches its Enable/window gate
  or its received-notification opt-in.
