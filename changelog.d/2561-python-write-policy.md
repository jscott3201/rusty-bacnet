---
section: Added
---
- **Python API:** `Device.write` and `Device.relinquish` refuse priorities 1, 2, 5 and 6 with `BacnetWriteDeniedError` unless `BACnetClient(write_policy=WritePolicy().allow_priorities([...]))` allows them; `write_property` stays unguarded (#2561).
