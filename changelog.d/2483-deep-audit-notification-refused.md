---
section: Changed
---
- **Wire:** An AuditNotification whose stored record would nest deeper than
  an AuditLogQuery-ACK carries is refused with its whole batch; before, it
  was stored and broke every query page that carried it (#2483).
