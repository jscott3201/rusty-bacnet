---
section: Changed
---
- **Python API:** SC credential errors name the file at fault in new words (a node's after `TLS config error:`, the hub's in its `BacnetError`). The hub checks the CA file first, and nodes and hub check the CA's certificates one at a time (#2600).
