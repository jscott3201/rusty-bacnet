---
section: Changed
---
- A dropped `tick_schedules` or `poll_trend_logs` that has taken the database
  may still make its whole pass, and the Command and Channel runs a dropped
  tick was making end when its task next runs, not in the drop (#2781).
