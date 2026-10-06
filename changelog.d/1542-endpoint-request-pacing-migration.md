---
section: Migration notes
---
- **SessionConfig (Rust API, #1542):** a `SessionConfig` literal needs
  `min_request_interval_ms` (or `..SessionConfig::default()`).
