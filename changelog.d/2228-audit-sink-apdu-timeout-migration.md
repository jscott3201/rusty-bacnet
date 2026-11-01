---
section: Migration notes
---
- **Audit sink helpers (Rust API, #2228):** `handle_audit_notification(db, sink, request)` becomes `(db, device, sink, request)`: pass the Device your identity names, such as `identity.device_oid()`. A missing Device still refuses the batch with DEVICE / OPERATIONAL_PROBLEM. `handle_confirmed_audit_notification` is removed; call `handle_audit_notification`.
