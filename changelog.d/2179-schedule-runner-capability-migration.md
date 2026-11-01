---
section: Migration notes
---
- **ScheduleRunner (Rust API, #2179):** move the four Schedule hooks from
  `impl BACnetObject` into `impl ScheduleRunner` unchanged, and lend it from
  `schedule_runner()` and `schedule_runner_mut()`. `s.tick_schedule(..)`
  becomes `s.schedule_runner_mut().and_then(|r| r.tick_schedule(..))`; on a
  concrete `ScheduleObject`, importing `ScheduleRunner` keeps direct calls
  ([details](docs/rust-api.md#capabilities)).
