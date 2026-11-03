---
section: Added
---
- `bacnet_objects::snapshot` makes the log and Timer/Channel written-configuration snapshot formats public, with `encode` and `decode` that need only `alloc`, so a flash or EEPROM backend stores the octets the file backends write (#2047).
