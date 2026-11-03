---
section: Migration notes
---
- **Written configuration (Rust API, #2331):** `WriteConfigSnapshot` is
  `#[non_exhaustive]` and gains `Reporting(ReportingWriteConfig)`, so a
  `match` on it takes a wildcard arm; build a `ReportingWriteConfig` from
  `Default` and set its fields. A custom `WriteConfigPersistence` stores it
  like the other kinds (`snapshot::write_config::encode` gives its octets).
  Older files still load.
