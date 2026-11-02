---
section: Changed
---
- **Breaking (Rust API):** `ServerConfig` has a new `object_types`
  field, an `ObjectTypeRegistry` holding the application's CreateObject
  constructors (#2173) and the types it keeps from deletion (#2179).
