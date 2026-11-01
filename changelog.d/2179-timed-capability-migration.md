---
section: Migration notes
---
- **Timed (Rust API, #2179):** move clock hooks into `impl Timed`, return it
  from `timed()`; one `bind_clocks(&Clocks)` replaces the three binds. Move
  `profile.expire(now)` into `DurableWrites::expire_staged`;
  `durable_writes_internal` is `durable_writes`. `advance_time_internal` and
  `lighting_blink_count_internal` are gone; `MonotonicClock`,
  `DeadlineWaker` and `DurableWrites` are public. `set_monotonic_clock` and
  `set_deadline_waker` drop `_internal`. New: `start_outputs`
  ([Capabilities](docs/rust-api.md#capabilities)).
