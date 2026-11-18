---
section: Migration notes
---
- **Device (Rust API, #2669):** `DeviceObject::set_object_list` and
  `DeviceObject::set_database_revision` are gone: the database keeps both for
  the local Device. Drop list-building code; read the list with
  `ObjectDatabase::object_list`, and keep a saved revision with
  `ObjectDatabase::set_database_revision` once the database is built.
