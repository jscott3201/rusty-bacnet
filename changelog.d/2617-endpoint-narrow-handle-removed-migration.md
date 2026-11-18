---
section: Migration notes
---
- **Endpoint server role handle (Rust API, #2617):** use `server()`, which
  borrows the full server, where you used `ServerRoleHandle`, and
  `is_running()` for `is_session_alive()`. For `RESPONDER_SERVICES` and
  `RESPONDER_OPT_IN_SERVICES` read the Device's Protocol_Services_Supported,
  or `bacnet_objects::device::EXECUTED_SERVICES`
  ([table](docs/rust-api.md#the-narrow-server-role-is-gone)).
