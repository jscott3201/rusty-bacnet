---
section: Added
---
- **Breaking (Rust API):** `ServerConfig::notification_retries` (default 3)
  sets how often the server sends a confirmed notification or remote write
  again; a server on an endpoint's seat takes the session's (#2617).
