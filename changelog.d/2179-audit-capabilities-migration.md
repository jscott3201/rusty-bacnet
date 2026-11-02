---
section: Migration notes
---
- **Audit hooks (Rust API, #2179):** a custom Audit Log implements the hidden
  `AuditLog` and lends it from `audit_log()` and `audit_log_mut()`; its
  required `storage()` returns what `audit_log_storage_internal` did, so a
  sink without storage is no longer possible. Callers read
  `audit_log().map(|log| log.storage())`. Reporter settings go through
  `audit_reporter_mut()` and `AuditReporterAuthority::configure`, which a
  wrapper can no longer intercept.
