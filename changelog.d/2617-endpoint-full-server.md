---
section: Changed
---
- **Breaking (Rust API):** `EndpointSession::with_server(builder)` has the
  session build, own and stop first a full `BACnetServer` on its address;
  `server()` now borrows that server, and the narrow server role's handle is
  `responder()` and `cloned_responder_handle()` (#2617).
