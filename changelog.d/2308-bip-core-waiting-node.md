---
section: Changed
---
- **Breaking (Rust API):** A `bacnet-bip-core` foreign node can start before its BBMD is known: `Foreign::bbmd` is an `Option`, and with `None` the node waits, holding broadcasts, until a `retarget` names its BBMD (#2308).
