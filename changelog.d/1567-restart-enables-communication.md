---
section: Fixed
---
- **Wire:** Accepted WARMSTART and COLDSTART requests end DISABLE_INITIATION immediately, cancel its expiry timer and resume queued COV notifications, even if the reply later fails (#1567).
