---
section: Migration notes
---
- **Device core writes (Rust API, #2178):**
  `bacnet_server::handlers::handle_write_property` is gone: write through
  `bacnet_device_core` (`responder::Responder::write`, or
  `write::write_property` then `write::end_unmade` for the bare handler's
  behaviour) or the server's `write_local`. `Responder::handle` now returns
  `Written`; read its `outcome`.
