---
section: Added
---
- **Python API:** `BACnetServer`, `BipEndpoint`, `ScEndpoint` and
  `MstpEndpoint` take `device_storage_path`: the Device keeps a written
  Description, and with `clock_zone_writable` a written zone, in that file
  and serves it again after a restart (#2730).
