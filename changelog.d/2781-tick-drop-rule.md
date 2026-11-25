---
section: Changed
---
- A dropped `tick_schedules` or `poll_trend_logs` that has taken the database
  may still make its whole pass. If a tick's task is mid-poll on another
  thread as the tick is dropped, one write already under way may still land
  and the next tick may find that Command or Channel busy (#2781).
