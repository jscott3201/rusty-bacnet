---
section: Changed
---
- **Python API:** `get_device()` and the `_from_device`/`_to_device` methods
  raise the new `BacnetAmbiguousDeviceError` (`instance`, `candidates`) when
  two addresses claim the instance, and `discovered_devices()` lists both
  (#2560).
