---
section: Migration notes
---
- **Reserved instance (Rust API, #2171):** `ObjectDatabase::add` refuses an
  object at instance 4194303, a Device included, and `DeviceIdentity::new`
  returns `Error::Range` for it. Give the object or identity an instance from
  0 to 4194302.
