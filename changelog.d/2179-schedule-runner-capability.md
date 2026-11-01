---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s Schedule hooks, `tick_schedule`,
  `take_owed_schedule_writes`, `complete_schedule_write` and
  `retry_refusals_naming`, move to a public `ScheduleRunner` capability lent
  through `schedule_runner()` (#2179).
