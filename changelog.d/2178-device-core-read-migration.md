---
section: Migration notes
---
- **Device core (Rust API, #2178):** `ReadPropertyMultipleBudget` moved from
  `bacnet_server::server` to `bacnet_device_core::read`; update imports.
  `bacnet_server::handlers::handle_read_property` and `handle_who_has` are
  gone: read through `bacnet_device_core` (`responder::Responder` or the
  staged `read` functions with `NoOverlay`) or the server's `read_local`.
