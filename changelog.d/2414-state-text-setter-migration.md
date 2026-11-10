---
section: Migration notes
---
- **State_Text kept (Rust API, #2414):** on a Multi-state Input, Output or
  Value keeping its reporting settings, `set_number_of_states` no longer
  changes a count a peer set through State_Text; call
  `clear_written_reporting` first to apply the application's own. A custom
  `WriteConfigPersistence` must keep `AlgorithmWriteConfig`'s new
  `state_text`. An older release refuses a file holding it: clear before a
  downgrade.
