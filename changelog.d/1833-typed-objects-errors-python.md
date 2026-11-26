---
section: Changed
---
- **Python API:** object, Device identity and COV policy refusals keep their
  exception types, but a message no longer starts with `encoding error: ` (a
  `value out of range: ` one keeps its prefix) (#1833).
