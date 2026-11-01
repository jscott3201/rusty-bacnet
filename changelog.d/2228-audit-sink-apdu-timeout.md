---
section: Fixed
---
- **Breaking (Rust API):** An Audit Log sink takes APDU_Timeout from the identity's Device, not the lowest one; `handle_audit_notification` takes that Device's identifier, and the `handle_confirmed_audit_notification` alias is gone (#2228).
