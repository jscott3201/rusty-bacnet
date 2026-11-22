---
section: Changed
---
- **Breaking (Rust API):** `TransportPort::stop` and `NetworkLayer::stop` take `&self`, so a link stops while others hold it, and every later send fails with a `NotConnected` transport error. `EndpointSession::stop` with a live `client()` now finishes in one call (#2617).
