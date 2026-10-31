---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject::property_metadata` is required, and
  the fallbacks for an object without rows are gone: the server reads every
  object's rows for presence, ReadPropertyMultiple's ALL, REQUIRED and
  OPTIONAL, indexed writes and the PICS (#2181).
