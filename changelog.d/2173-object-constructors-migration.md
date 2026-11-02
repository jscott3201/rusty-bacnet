---
section: Migration notes
---
- **CreateObject constructors (Rust API, #2173):** exhaustive `ServerConfig`
  literals need `object_types: ObjectTypeRegistry::default()`, or end
  with `..ServerConfig::default()`.
