---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::Client::discover` takes one
  `Discover` (scope, range, wait), a local broadcast unless it names a
  network or every network; `Scope` replaces `WhoIsScope` and also sends on a
  low-level client (#2560).
