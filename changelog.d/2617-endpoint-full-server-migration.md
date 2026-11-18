---
section: Migration notes
---
- **Endpoint server role (Rust API, #2617):** replace `server()` with
  `responder()` and `cloned_server_handle()` with `cloned_responder_handle()`
  where you meant the narrow server role. Check every `server().is_some()` or
  `is_none()`: it still compiles but now asks about a full server
  `with_server` built, so a role check like the shared-endpoint guide's
  printed `false`. Give such a session its database and Device identity,
  not the builder.
