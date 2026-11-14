---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::{safety, class, decode, bbmd}` move
  to `bacnet_tool`, the `Engine` BBMD reads become `bacnet_tool::bbmd`
  functions, and `bacnet_ops::destination` goes into the CLI;
  `engine::rpm_rows`, `format::timestamp_text` and `format::alarms::cell`
  become public (#2560).
