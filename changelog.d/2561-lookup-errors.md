---
section: Changed
---
- **Breaking (Python API):** a device that cannot be placed raises the new `BacnetLookupError`, also a `LookupError`, from the high-level calls and the typed `_from_device` calls, where the latter raised a bare `BacnetError`; `BacnetAmbiguousDeviceError` is now its subclass (#2561).
