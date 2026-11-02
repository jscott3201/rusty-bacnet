---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject` loses `is_writable_property`,
  `is_array_property` and `is_list_property`; free functions of the same
  names in `bacnet_objects::property_metadata` answer from the object's
  rows, and WriteProperty decodes a value by its row's value type (#2177).
