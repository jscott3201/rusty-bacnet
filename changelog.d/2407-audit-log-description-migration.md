---
section: Migration notes
---
- **Audit Log snapshot (Rust API, #2407):** `AuditLogSnapshot` gains a
  `description` field; set it to `None` in a struct literal. A slot holding
  one is schema version 4, which older builds refuse; slots without one
  keep their octets.
