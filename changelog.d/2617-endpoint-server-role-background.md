---
section: Changed
---
- **Breaking (wire):** An endpoint session's server runs its background work
  over the session's database: event detection and intrinsic reporting send
  notifications, Schedules write their targets and Trend Logs log, where a
  0.15.0 endpoint only served those objects' properties (#2617).
