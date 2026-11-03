---
section: Migration notes
---
- **Written configuration (Rust API, #2331):** a `match` on
  `WriteConfigSnapshot` handles `Reporting(ReportingWriteConfig)`. A custom
  `WriteConfigPersistence` stores it like the other kinds;
  `snapshot::write_config::encode` gives its octets. Older files still load.
