---
section: Migration notes
---
- **Lookup errors (Python API, #2561):** `except BacnetError` still catches an unknown device; code that told it apart by message can catch `BacnetLookupError`, which also catches `BacnetAmbiguousDeviceError`. Each of its `candidates`, a `(network, mac)` pair, is a `client.device()` target.
