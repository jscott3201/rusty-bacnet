---
section: Migration notes
---
- **Event Enrollment parameter setters (Rust API, #2331):** on an
  enrollment keeping its reporting settings, the three parameter setters no
  longer replace a value a peer wrote; call `clear_written_reporting` first
  to apply the application's own. A custom `WriteConfigPersistence` must
  keep `AlgorithmWriteConfig`'s new `event_parameters` and
  `fault_parameters`. An older release refuses an enrollment's file holding
  a parameter: clear before a downgrade.
