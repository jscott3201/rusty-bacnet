---
section: Migration notes
---
- **Audit Log (Rust API, #2483):** a `match` on `BACnetAuditLogDatum` needs
  an `Opaque` arm. An Audit Log snapshot an older build saved with a record
  nested deeper than an AuditLogQuery-ACK carries now loads without that
  record, logging a warning; each older record's sequence number moves up by
  the number of newer records dropped.
