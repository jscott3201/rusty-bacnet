---
section: Changed
---
- **Breaking (Python API):** `BACnetServer`'s nine analog, binary and multi-state builders share the endpoints' checks and refuse at the call a repeated identifier (once a silent replacement), a duplicate name (once a `start()` error), instance 4194303 and a non-finite Analog Input value (#2711).
