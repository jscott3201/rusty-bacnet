---
section: Added
---
- **Python API:** `EndpointClient`'s four reads take a `RoutedTarget(network,
  address, router)` where they take `address`, reaching a station behind a
  router through the endpoint's one socket; a `str` stays a station on the
  local link (#2612).
