---
section: Changed
---
- **Breaking (Python API):** `BACnetClient.who_is()` and `discover()` take
  `scope="local"` (the default), `"global"` or a network number, and with no
  scope send a local broadcast instead of a global one (#2560).
