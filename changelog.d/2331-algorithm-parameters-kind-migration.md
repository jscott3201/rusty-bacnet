---
section: Migration notes
---
- **Written configuration (Rust API, #2331):** `ReportingWriteConfig` gains
  the field `algorithm`. A custom `WriteConfigPersistence` that stores the
  fields itself must keep it; one that stores `snapshot::write_config::encode`'s
  octets needs no change. Older files still load. An older release refuses a
  file holding a parameter (kind 6): run `clear_written_reporting` before a
  downgrade.
