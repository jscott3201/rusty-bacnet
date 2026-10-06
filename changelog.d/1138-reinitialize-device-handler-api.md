---
section: Changed
---
- **Breaking (Rust API):** `ServerConfig` has a new `on_reinitialize` field,
  and `handle_reinitialize_device` returns the requested state instead of
  `()` (#1138).
