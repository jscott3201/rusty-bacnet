---
section: Migration notes
---
- **Transport stop (Rust API, #2617):** a `TransportPort` implementation changes `fn stop(&mut self)` to `fn stop(&self)`: keep what the stop takes behind a lock only start and stop use, and fail every later send (`bacnet_transport::port::stopped()` gives the error). Callers that held a transport or layer mutably only to stop it can drop the `mut`. Stop an `EndpointSession` once, with its `client()` alive or not: the "drop it, then stop again" error is gone.
