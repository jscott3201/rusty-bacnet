---
section: Fixed
---
- **Python API:** `Device.read_many` and `Device.read_object` raise `TypeError` when the call is
  made for one str, bytes or bytearray where a list is meant, as `inventory`, `points` and
  `watch` do (now bytearray too), instead of reading it a character at a time (#2561).
