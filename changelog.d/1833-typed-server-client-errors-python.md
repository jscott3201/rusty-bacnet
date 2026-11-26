---
section: Changed
---
- **Python API:** server and client refusals keep their exception types, but a
  message no longer starts with `encoding error: `; a request cut off by
  `stop()`, and an endpoint's audited request its stop ends (which said
  "endpoint shutdown"), raise the cut-off message (#1833).
