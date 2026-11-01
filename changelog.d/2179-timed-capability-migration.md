---
section: Migration notes
---
- **Timed (Rust API, #2179):** replace `bind_clock_internal` and the other
  clock hooks with `impl Timed` plus `fn timed(&mut self) -> Option<&mut dyn
  Timed> { Some(self) }`; the three binds become one `bind_clocks(&Clocks)`.
  Before: `self.profile.expire(now)` in `advance_monotonic_time_internal`.
  After: `fn expire_staged(&mut self, now)` in `impl DurableWrites`.
  `durable_writes_internal` is now `durable_writes`. See
  [Capabilities](docs/rust-api.md#capabilities).
