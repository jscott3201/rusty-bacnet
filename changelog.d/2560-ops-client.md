---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::Engine` is now `bacnet_ops::Client`
  (cheap to clone; `low_level()` for `client()`) in the module
  `bacnet_ops::client`, once `engine`. Its reads move to `Device` handles,
  and gates, turns and the no-RPM memory key on a device's address (#2560).
