---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::Client::discover` takes one
  `Discover` (scope, range, wait), a local broadcast unless it names a
  network or every network, and fails with `OpsError`; `Scope` replaces
  `WhoIsScope`, and `raw::who_is` sends one with no rate cap (#2560).
