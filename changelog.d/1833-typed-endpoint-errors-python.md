---
section: Changed
---
- **Python API:** endpoint, router and network-layer refusals keep their
  exception types and text, but a message the bindings pass on as the
  error's own text no longer starts with `encoding error: ` (#1833).
