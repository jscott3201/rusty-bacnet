---
section: Migration notes
---
- **Python API (#2711).** Catch a refused point registration at the `BACnetServer` `add_*` call, not at `start()`: `ValueError` for a duplicate name or identifier, `BacnetError` for instance 4194303, `BacnetProtocolError` for a NaN or infinite Analog Input value.
