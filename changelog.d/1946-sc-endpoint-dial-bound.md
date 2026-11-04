---
section: Changed
---
- `ScEndpoint`'s hub dial at start is bounded by `sc_connect_timeout_ms`,
  10 seconds by default, as `BACnetClient`'s is (#1946).
