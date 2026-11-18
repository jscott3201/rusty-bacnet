---
section: Migration notes
---
- **Endpoint server (Rust API, #2617):** `server()` borrows the session's
  `BACnetServer` in place of the narrow role's handle, so a call that used
  the handle now reaches the server; for another task, clone what it needs
  from the server, such as `server().database()`, in place of
  `cloned_server_handle()`. Give the session its database and Device
  identity, not the builder.
