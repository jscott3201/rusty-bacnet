---
section: Migration notes
---
- **Breaking (Python API).** For whole Network Port properties 414 and 415, handle `bdt` and `fdt` in place of `tagged` or an empty `list`. Read entry dictionaries from `.value`, including an empty list for an empty table. Compare wire bytes using `.octets` (#2690).
