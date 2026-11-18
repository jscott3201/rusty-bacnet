---
section: Migration notes
---
- **Endpoint server role (Rust API, #2617):** where you used the narrow server
  role's handle, call `responder()` for `server()` and
  `cloned_responder_handle()` for `cloned_server_handle()`. `server()` now
  returns `Option<&BACnetServer<T>>`: `Some` while a session given
  `with_server(builder)` before `start()` runs. Give that session its
  database and Device identity, not the builder.
