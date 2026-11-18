---
section: Changed
---
- **Breaking (Python API):** the endpoints take `BACnetServer`'s
  `mutation_policy`, `cov_policy` and `time_sync_policy`; without the first
  their server is read-only, without the last it ignores time
  synchronization. `device_write_policy`, `services` and the
  `responder_declined` status key go (#2617).
