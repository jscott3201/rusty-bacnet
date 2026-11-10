---
section: Migration notes
---
- **Alarm_Values setter (Rust API, #2331):** on a Multi-state Input or
  Value keeping its reporting settings, `set_alarm_values` no longer
  replaces a list a peer wrote; call `clear_written_reporting` first to
  apply the application's own. A custom `WriteConfigPersistence` must keep
  `AlgorithmWriteConfig`'s new `alarm_value` and `alarm_values`. An older
  release refuses a file holding either: clear before a downgrade.
