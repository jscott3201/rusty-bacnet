---
section: Changed
---
- Device-binding freshness, COV subscription lifetimes, Life Safety operation
  replay and the confirmed-event repeat window now read tokio's clock through
  the helper the server's rate limiters use, so paused tests step them
  exactly; production timing is unchanged (#1556).
