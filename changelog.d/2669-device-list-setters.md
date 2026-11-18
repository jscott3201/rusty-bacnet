---
section: Removed
---
- **Breaking (Rust API):** `DeviceObject::set_object_list` and
  `DeviceObject::set_database_revision`: the database keeps the local
  Device's Object_List and Database_Revision (`ObjectDatabase::object_list`,
  `ObjectDatabase::set_database_revision`) (#2669).
