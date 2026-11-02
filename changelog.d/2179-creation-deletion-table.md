---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject`'s `creation_only_properties`,
  `initialize_property` and `is_deleteable` move to the type: a new
  `Creatable` trait and the CreateObject table answer creation, and
  `ObjectConstructors::deletes` answers deletion (#2179).
