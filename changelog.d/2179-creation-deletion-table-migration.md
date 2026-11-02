---
section: Migration notes
---
- **Creation and deletion (Rust API, #2179):** move a type's
  `creation_only_properties` and `initialize_property` into
  `impl Creatable for MyType` (`CREATION_ONLY_PROPERTIES` and
  `initialize_property`; an empty `impl` when it has none); a CreateObject
  constructor for it is registered by returning `MyType` itself. Replace an
  `is_deleteable() -> false` override with the builder's or
  `ObjectTypeRegistry`'s `keep_from_deletion(type)`
  ([details](docs/rust-api.md#creation-and-deletion)).
