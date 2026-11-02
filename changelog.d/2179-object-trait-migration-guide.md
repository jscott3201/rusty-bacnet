---
section: Migration notes
---
- **BACnetObject (Rust API, #2179):** the trait is now the core a custom
  object implements plus one accessor per capability.
  [Migrating custom objects to 0.16](docs/rust-api.md#migrating-custom-objects-to-016)
  maps every removed or renamed method of the series to its new home.
