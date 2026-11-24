---
section: Changed
---
- A dropped `tick_schedules` or `poll_trend_logs` that has taken the database
  may still make its whole pass. On a multi-thread runtime, a dropped tick
  whose runs a task is polling at that moment ends them as that poll returns,
  not in the drop (#2781).
