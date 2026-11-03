---
section: Added
---
- **Python API:** `EndpointSession::report_device_reset_local()` and the
  Python endpoints' `report_device_reset_local()` record a reset the
  application makes on its own, as `BACnetServer`'s does, through the
  session's source Audit Reporter (#1837).
