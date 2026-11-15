---
section: Changed
---
- **Breaking (Python API):** `await endpoint.client()` on `BipEndpoint`,
  `ScEndpoint` and `MstpEndpoint` returns a `BACnetClient` on the endpoint's
  one address, with every call a standalone client has; `EndpointClient` is
  removed (#2617).
