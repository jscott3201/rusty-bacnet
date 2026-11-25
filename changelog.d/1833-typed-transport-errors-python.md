---
section: Changed
---
- **Python API:** transport refusals keep their exception types and text, but
  a message the bindings pass on as the error's own text no longer starts
  with `encoding error: ` or `value out of range: ` (#1833).
