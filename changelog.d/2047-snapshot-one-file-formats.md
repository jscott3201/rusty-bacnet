---
section: Added
---
- `bacnet_objects::snapshot` gains the Notification Class, Notification Forwarder, Tags, colour and Access Rights file formats, with `encode` and `decode` that need only `alloc`, so a custom backend stores the octets the file backends write (#2047).
