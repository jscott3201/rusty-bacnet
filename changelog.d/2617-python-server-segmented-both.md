---
section: Changed
---
- **Breaking (wire):** Python `BACnetServer()` segments both ways by default, as the Rust server
  and the endpoints do: its Device states SEGMENTED_BOTH, and it reassembles segmented requests
  and sends long answers in segments (#2617).
