---
section: Changed
---
- **Breaking (Rust API):** Starting a server with `ServerConfig::cov_retry_timeout_ms` of 0 now fails with an error naming the field, instead of giving every confirmed report a deadline at its own start; any non-zero value is accepted (#2412).
