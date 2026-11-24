---
section: Changed
---
- **Breaking (Python API):** Every `BACnetServer.add_*` builder refuses at the call a repeated identifier (once a silent replacement), a duplicate name and instance 4194303, as the point builders do (#2748).
