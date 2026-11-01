---
section: Migration notes
---
- **Audit sink helpers (Rust API, #2228):** `handle_audit_notification(db, sink, request)` and `handle_confirmed_audit_notification(db, sink, request)` become `(db, device, sink, request)`. Pass the Device your identity names, for example `identity.device_oid()`; a missing Device still refuses the batch with DEVICE / OPERATIONAL_PROBLEM.
