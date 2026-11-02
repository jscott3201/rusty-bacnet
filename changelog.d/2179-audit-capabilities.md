---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s Audit Log hooks, the public
  `audit_log_storage_internal` and `audit_log_notification_sink_internal`
  among them, move behind a hidden `AuditLog` capability, and the Audit
  policy and Reporter hooks behind hidden accessors (#2179).
