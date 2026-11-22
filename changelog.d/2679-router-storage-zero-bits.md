---
section: Fixed
---
- A router's empty storage is all zero bits whatever the size of its
  duplicate record, as `bacnet-router-core` documents: firmware that keeps a
  record still has the storage in `.bss`, and a program with a
  `BACnetRouter` no longer carries 150 KB of read-only data for it (#2679).
