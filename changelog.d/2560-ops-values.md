---
section: Changed
---
- **Breaking (Rust API):** `bacnet-ops` reads return a `Value` (typed data,
  text and the octets sent) or `Readings`, and its calls fail with
  `OpsError`; `RowError` is `PropertyError`, and a refused target is no
  longer an encoding or range error (#2560).
