---
section: Changed
---
- **Breaking (Rust API):** `EndpointSession::with_server(builder)` has the
  session build, own and stop first a full `BACnetServer` on its address;
  `server()` now borrows that server, and `cloned_server_handle()` is gone
  (#2617).
