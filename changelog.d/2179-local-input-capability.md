---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s local-input hooks become one
  `LocalInput` capability, lent through `local_input()` and taking a
  `LocalInputValue`; the Life Safety and access-event-batch hooks move behind
  hidden `life_safety()` and `access_events()` accessors (#2179).
