---
section: Changed
---
- **Python API:** object, Device identity and COV policy refusals keep their
  exception types, but a message no longer starts with `encoding error: `, nor
  with `value out of range: ` for an empty storage path or audit batch or an
  exhausted counter; other range refusals keep that prefix (#1833).
