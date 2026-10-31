---
section: Changed
---
- **Python API:** `BACnetServer`, `BipEndpoint`, `ScEndpoint` and
  `MstpEndpoint` raise `ValueError` at construction for a `device_instance`
  of 4194303 (#2171).
