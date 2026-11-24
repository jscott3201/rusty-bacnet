---
section: Fixed
---
- `tick_schedules`, `poll_trend_logs` and a server's start-up Staging writes no
  longer wait for the database in their caller's task, so a tick or start left
  unpolled holds up no request; a tick dropped before its turn makes no pass
  (#2781).
