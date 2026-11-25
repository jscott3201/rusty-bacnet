---
section: Changed
---
- **Python API:** server and client refusals keep their exception types, but a
  message no longer starts with `encoding error: `; a request cut off by
  `stop()` keeps its message, which an endpoint's audited request its stop
  ends now raises too, where it said "endpoint shutdown" (#1833).
