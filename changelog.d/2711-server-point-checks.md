---
section: Changed
---
- **Breaking (Python API):** `BACnetServer`'s nine analog, binary and multi-state builders share the endpoints' registration code: a duplicate name or identifier, instance 4194303 and a non-finite Analog Input value are refused at the call, not at start (#2711).
