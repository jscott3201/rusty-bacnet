---
section: Fixed
---
- A `write_local` or `purge_audit_log` call dropped during a durable save, or while waiting for the database guard, now releases its stage at once, not about 10 s later (#2493).
