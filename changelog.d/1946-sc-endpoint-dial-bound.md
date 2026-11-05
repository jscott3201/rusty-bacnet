---
section: Changed
---
- **Python API:** `ScEndpoint.start()` now gives up a hub dial after
  `sc_connect_timeout_ms` (10 s by default), as `BACnetClient` does; before,
  it waited as long as the OS allowed (#1946).
