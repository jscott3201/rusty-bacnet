---
section: Changed
---
- **Breaking (Rust API):** the client's discovery table is keyed by address
  (network and MAC): two addresses claiming one instance each keep a row,
  reachable by address, and `get_device` and the `_from_device`/`_to_device`
  calls refuse that instance with the new `Error::AmbiguousDevice` (#2560).
