---
section: Changed
---
- **Python API:** server and client refusals keep their exception types and
  text, but a message the bindings pass on no longer starts with
  `encoding error: `, such as a time-sync policy's `ValueError`; a request cut
  off by `stop()` still raises its own message (#1833).
