---
section: Added
---
- **Python API:** `EndpointClient`'s four reads take a `RoutedTarget(network,
  address, router=None)` for a station behind a router: through `router`, or
  with none by one local broadcast naming the network; a `str` stays a
  station on the local link (#2612).
