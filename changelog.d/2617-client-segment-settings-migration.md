---
section: Migration notes
---
- **Client settings (Rust API, #2617):** a `ClientConfig` struct literal names the new
  `segmented_requests` and `apdu_segment_timeout_ms` fields, or ends with
  `..ClientConfig::default()`.
