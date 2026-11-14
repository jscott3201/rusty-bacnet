---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::Engine` is now `bacnet_ops::Client`,
  cheap to clone, with `low_level()` for `client()`; its reads move to
  `Device` handles, and request gates, turns and the no-RPM memory key on a
  device's address, so one device named two ways waits at one gate (#2560).
