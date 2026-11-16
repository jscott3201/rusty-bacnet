---
section: Changed
---
- **Breaking (Rust API, Python API):** Endpoint sessions default to a 6000 ms
  APDU timeout and 3 retries, which time the node's client requests, a seated
  server's confirmed notifications and its Device's APDU_Timeout and
  Number_Of_APDU_Retries (#2617).
