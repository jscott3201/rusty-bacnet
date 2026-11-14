---
section: Changed
---
- **Breaking (Rust API):** `bacnet-ops` reads return a `Value` (typed data,
  text and the octets sent), `Readings` or an `ObjectRead`, and its calls
  fail with `OpsError`; the raw readers move to `bacnet_ops::raw`, and
  `RowError` is `PropertyError` (#2560).
