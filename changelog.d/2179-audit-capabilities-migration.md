---
section: Migration notes
---
- **Audit hooks (Rust API, #2179):** a custom Audit Log implements the hidden
  `AuditLog` (its `storage()` is the old `AuditLogStorage`) and lends it from
  `audit_log()` and `audit_log_mut()`; callers read
  `audit_log().map(|log| log.storage())` and
  `audit_log_mut().and_then(|log| log.notification_sink())`. Reporter settings
  go through `audit_reporter_mut()` and `AuditReporterAuthority::configure`.
