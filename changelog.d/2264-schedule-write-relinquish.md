---
section: Changed
---
- **Breaking (Rust API):** `ScheduleWrite` is `#[non_exhaustive]`, built with
  `ScheduleWrite::new`, and gains a `relinquish` flag marking the NULLs that
  give up slots a Schedule held (#2264).
