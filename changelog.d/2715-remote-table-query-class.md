---
section: Added
---
- `bacnet-router-core` asks its host about a routing-table query that
  carries an SNET, as `ControlClass::InitQuery`, so a host can refuse or
  limit answers sent toward another network. A host that allows every class
  answers as before, and a query with no SNET is never asked about (#2715).
