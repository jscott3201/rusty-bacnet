---
section: Changed
---
- **Python API:** the bindings' own lifecycle checks (a client, server,
  endpoint, router or hub not started, stopping, stopped or started twice)
  raise `BacnetStateError` with its `state`, as the Rust stack's do. Not
  breaking: it is still a `RuntimeError`, and the messages are unchanged
  (#2819).
