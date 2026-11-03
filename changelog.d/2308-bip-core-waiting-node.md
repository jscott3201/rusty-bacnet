---
section: Changed
---
- **Breaking (Rust API):** A `bacnet-bip-core` foreign node can start before its BBMD is known: `Foreign::bbmd` is an `Option`, and `Foreign::waiting` (`None`) starts a node that holds broadcasts until a `retarget` names its BBMD (#2308).
