---
section: Fixed
---
- **Breaking (Rust API):** Every built-in event reporter now refuses every
  event transition commit while Event_Detection_Enable is FALSE, NORMAL
  targets included, with `DetectionDisabled`; the server never made such a
  commit, so nothing changes on the wire (#2271).
