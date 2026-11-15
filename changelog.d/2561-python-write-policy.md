---
section: Added
---
- **Python API:** `relinquish` refuses priorities 1, 2, 5 and 6 with
  `BacnetWriteDeniedError` unless
  `BACnetClient(write_policy=WritePolicy().allow_priorities([...]))` allows
  them; `write_property` stays unguarded (#2561).
