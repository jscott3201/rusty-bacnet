---
section: Changed
---
- **Breaking (Rust API):** `RouterPort` is `#[non_exhaustive]` and built with
  `RouterPort::new(transport, network_number)`, so a later per-port setting
  isn't a breaking change (#2684).
