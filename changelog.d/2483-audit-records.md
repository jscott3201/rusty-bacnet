---
section: Changed
---
- **Breaking (Rust API):** `BACnetAuditLogDatum` gains `Opaque(OpaqueChoice)`,
  and the Audit Log record encoder, and so `add_record`, stored audit
  notifications and log restore, refuse a record nested deeper than an
  AuditLogQuery-ACK carries (#2483).
