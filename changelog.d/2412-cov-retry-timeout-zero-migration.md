---
section: Migration notes
---
- **Server (Rust API, #2412):** a `ServerConfig` with `cov_retry_timeout_ms: 0` no longer starts; set it to at least 1, or leave the 3,000 ms default.
