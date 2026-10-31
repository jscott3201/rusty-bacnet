---
section: Migration notes
---
- **Device property list (Rust API, #2174):** `DeviceObject::property_list()`
  no longer names Property_List. Code that looked for it there should look in
  `property_metadata()`, which still has the row, as for any other type.
