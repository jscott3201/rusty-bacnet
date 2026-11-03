---
section: Added
---
- **Rust and Python API:** `EndpointSession::report_device_reset_local()`
  records a reset the application makes itself through the session's
  source Audit Reporter, as `BACnetServer`'s does; the Python endpoints gain
  the method, which records nothing until Python can configure that Reporter
  (#1837).
