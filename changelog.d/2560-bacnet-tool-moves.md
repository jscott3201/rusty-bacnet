---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::{safety, class, bbmd}` move to
  `bacnet_tool`, the `Engine` BBMD reads become
  `bacnet_tool::bbmd` functions, `bacnet_ops::destination` moves to the
  CLI, and `rpm_rows`, `timestamp_text` and `alarms::cell` in
  `bacnet_ops` are public (#2560).
