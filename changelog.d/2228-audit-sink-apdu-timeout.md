---
section: Fixed
---
- **Breaking (Rust API):** An Audit Log sink takes APDU_Timeout from the identity's Device, not the lowest one; `handle_audit_notification` and `handle_confirmed_audit_notification` take that Device's identifier (#2228).
