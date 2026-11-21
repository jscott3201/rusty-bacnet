---
section: Migration notes
---
- **Python API:** Breaking: an invalid B/IP `interface` given to `BACnetClient` or `BACnetServer` now raises `ValueError` at construction, where `start()` used to raise `RuntimeError`; catch it there (#2614).
