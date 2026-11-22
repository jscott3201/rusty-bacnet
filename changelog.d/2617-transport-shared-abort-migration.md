---
section: Migration notes
---
- **Transport abort (Rust API, #2617):** a `TransportPort` implementation
  changes `fn abort(&mut self)` to `fn abort(&self)`: do at once, through
  the locks its stop uses, what the stop does, without waiting or sending
  a goodbye, and fail every later send with
  `bacnet_transport::port::stopped()`.
