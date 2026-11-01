---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s local-input hooks become one
  `LocalInput` capability, lent through `local_input()` and taking a
  `LocalInputValue`; the Life Safety hooks become a `LifeSafety` capability
  lent through `life_safety()`. `AccessControlInput` is `#[non_exhaustive]`
  (#2179).
