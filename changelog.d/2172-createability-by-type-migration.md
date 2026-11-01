---
section: Migration notes
---
- **Createability (Rust API, #2172):** delete `is_createable` overrides and
  forwarding impls. To ask whether CreateObject builds a type, call
  `ObjectConstructors::creates(object_type)` on the server configuration's
  registry; to make a custom type creatable, register a constructor for it.
