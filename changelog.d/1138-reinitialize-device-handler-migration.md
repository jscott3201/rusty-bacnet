---
section: Migration notes
---
- **ReinitializeDevice (Rust API, #1138):** exhaustive `ServerConfig` literals
  need `on_reinitialize: None`. Callers of `handle_reinitialize_device` get the
  decoded `ReinitializedState` back in place of `()`.
