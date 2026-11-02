---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s log hooks move behind a hidden
  `LogBuffer` capability lent through `log_buffer()`/`log_buffer_mut()`, and
  the Trend Log collecting and Event Log received-notification checks leave
  the trait; these hooks are not public API yet (#2179).
