---
section: Added
---
- **Python API:** `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take a
  keyword-only `min_request_interval_ms`, validated and applied to their
  `EndpointClient` as `BACnetClient`'s keyword is (#1542).
