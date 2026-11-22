---
section: Changed
---
- **Breaking (Rust API):** `TransportPort::abort` takes `&self`, and the new
  `NetworkLayer::abort` ends a shared layer at once, so a dropped
  `EndpointSession` aborts its link even while its `client()` lives: the
  port is let go of and the client's sends fail (#2617).
