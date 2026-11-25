---
section: Changed
---
- **Python API:** endpoint, router and network-layer refusals keep their
  exception types and text, but a message the bindings pass on no longer
  starts with `encoding error: `; `routing_table()` on a router-application
  client still raises `ValueError` with the same text (#1833).
