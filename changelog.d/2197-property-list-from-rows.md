---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject` loses `property_list` and
  `required_properties`: the metadata rows are an object's only property
  list, and free functions in `bacnet_objects::property_metadata` derive
  Property_List, the required set and presence from them (#2197).
