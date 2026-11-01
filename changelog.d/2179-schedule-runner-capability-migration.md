---
section: Migration notes
---
- **ScheduleRunner (Rust API, #2179):** move the four Schedule hooks from
  `impl BACnetObject` into `impl ScheduleRunner` unchanged, and lend it from
  `schedule_runner(&mut self)`. `s.tick_schedule(..)` becomes
  `s.schedule_runner().and_then(|r| r.tick_schedule(..))`, and
  `retry_refusals_naming` is reached the same way. On a concrete
  `ScheduleObject`, importing `ScheduleRunner` keeps direct calls
  ([details](docs/rust-api.md#capabilities)).
