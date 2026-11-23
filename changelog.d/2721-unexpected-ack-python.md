---
section: Fixed
---
- **Python API:** a write the device acknowledges with a Complex-ACK, or a
  read it acknowledges with a Simple-ACK, raises `BacnetError` naming the
  service and both kinds, where a write used to succeed and a read failed
  to decode (#2721).
