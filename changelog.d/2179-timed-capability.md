---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s time and clock hooks move to a
  `Timed` capability that an object lends through `timed()`, and staged-write
  expiry moves to `DurableWrites::expire_staged`, so an object that only
  stages writes is no longer timed (#2179).
