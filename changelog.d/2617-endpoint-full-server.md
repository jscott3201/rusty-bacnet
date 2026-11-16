---
section: Added
---
- **Rust API:** `EndpointSession::full_server(builder)` runs a full
  `BACnetServer` on the session's one address beside its full client, and
  `ServerBuilder::seat` runs one on any node's server seat; the session's
  `server_seat_counters()` count what the seat dropped (#2617).
