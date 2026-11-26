---
section: Changed
---
- **Python API:** a client, server, endpoint, router or hub called in the
  wrong state raises `BacnetStateError`, messages unchanged (#2819).
  `except RuntimeError` still catches it; `except BacnetError` now does too,
  so a retry loop on `BacnetError` should let `BacnetStateError` through.
