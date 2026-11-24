---
section: Migration notes
---
- **Python API (#2711).** A `BACnetServer` point builder raises at the call: `ValueError` for a repeated identifier (which used to replace the earlier object) or a duplicate name (which `start()` raised), `BacnetError` for instance 4194303, `BacnetProtocolError` for a non-finite Analog Input value.
