---
section: Added
---
- **Rust and Python API:** `EndpointSession::report_device_reset_local()`
  records a reset the application makes itself through the session's
  source Audit Reporter, as `BACnetServer`'s does; in Python, the
  endpoint's server records it so, through `await endpoint.server()`
  (#1837, #2617).
