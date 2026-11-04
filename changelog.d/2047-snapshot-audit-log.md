---
section: Added
---
- `bacnet_objects::snapshot::audit_log` makes the Audit Log's two-slot snapshot format public, with `encode`, `decode` and `newest` that need only `alloc`, so a custom backend stores the octets and makes the slot choice `FileAuditLogPersistence` does (#2047).
