---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s COV hooks move to a public
  `CovReporting` capability lent through `cov()`, where `None` means the
  object takes no COV subscription. The unused `set_overridden` hook is
  removed (#2179).
