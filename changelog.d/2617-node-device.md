---
section: Changed
---
- **Breaking (Rust API):** `EndpointSession::with_server` needs a session
  identity and serves the node's Device: the smaller largest APDU, the
  session's APDU timing and, on a `Both` session, the client's services too;
  a contradicting builder or database is refused (#2617).
