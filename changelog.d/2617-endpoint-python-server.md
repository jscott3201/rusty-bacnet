---
section: Changed
---
- **Breaking (Python API):** `await endpoint.server()` on `BipEndpoint`,
  `ScEndpoint` and `MstpEndpoint` returns the `BACnetServer` the endpoint
  runs on its one address, with a standalone server's calls; the endpoint
  starts and stops it. `EndpointServer` is removed (#2617).
