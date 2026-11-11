---
section: Changed
---
- **Breaking (Rust API):** `LocalCommandSource`, the source a local write
  names, moves from `bacnet-server` to `bacnet-device-core`'s
  `write::local`, which firmware shares (#2446).
