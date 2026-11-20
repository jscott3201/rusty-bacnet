---
section: Migration notes
---
- **Breaking (Python API).** Handle an invalid B/IP `interface` at `BACnetClient` or `BACnetServer` construction. It now raises `ValueError` there. Previously, starting the client or server raised `RuntimeError` (#2614).
