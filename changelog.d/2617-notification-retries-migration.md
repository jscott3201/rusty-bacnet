---
section: Migration notes
---
- **Server config (Rust API, #2617):** a `ServerConfig` written out field by
  field adds `notification_retries: 3` (the value the server always used), or
  ends with `..ServerConfig::default()`.
