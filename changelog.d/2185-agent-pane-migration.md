---
section: Migration notes
---
- **Watch hub (Rust API, #2185):** `bacnet_ops::watch::WatchOptions` has a new
  `agent` field; a struct literal that lists every field adds `agent: false`
  (or ends with `..WatchOptions::default()`).
