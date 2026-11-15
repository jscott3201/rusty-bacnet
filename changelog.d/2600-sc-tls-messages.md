---
section: Changed
---
- **Python API:** SC credential errors name the file at fault in new words (a node's after `TLS config error:`, the hub's in its `BacnetError`); the hub checks the CA file first, the CA's certificates one at a time; `inspect_sc_pem` refuses a key a connect refuses (#2600).
