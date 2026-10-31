---
section: Migration notes
---
- **Device instance (Python API, #2171):** `BACnetServer`, `BipEndpoint`,
  `ScEndpoint` and `MstpEndpoint` built with `device_instance=4194303` now
  raise `ValueError` at construction, where a server used to fail at
  `start()`. Use an instance from 0 to 4194302.
