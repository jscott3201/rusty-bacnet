---
section: Changed
---
- **Breaking (Python API):** the endpoints take `BACnetServer`'s
  `mutation_policy` and `cov_policy`, and their server is read-only without a
  policy; `device_write_policy`, `services` and the `responder_declined`
  status key are removed (#2617).
