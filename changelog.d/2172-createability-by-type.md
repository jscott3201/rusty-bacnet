---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject::is_createable` is removed:
  createability belongs to the object type, answered by
  `ObjectConstructors::creates`, and the PICS now lists every createable
  type, with or without an instance (#2172, #2173).
