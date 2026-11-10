---
section: Migration notes
---
- **Access Door and Zone list setters (Rust API, #2331):** on an object
  keeping its reporting settings, the three list setters no longer replace
  a list a peer wrote; call `clear_written_reporting` first to apply the
  application's own. A custom `WriteConfigPersistence` must keep
  `AlgorithmWriteConfig`'s new `fault_values` and `masked_alarm_values`. An
  older release refuses a door's or zone's file holding a parameter: clear
  before a downgrade.
