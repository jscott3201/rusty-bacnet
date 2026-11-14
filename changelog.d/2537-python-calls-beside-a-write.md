---
section: Fixed
---
- **Python API:** A `BACnetServer` call no longer waits behind a local write
  that waits for its event notifications, such as one for an unbound Device
  recipient waiting for its I-Am; `stop()` still lets such a write return
  first, within `cov_retry_timeout_ms` (#2537).
