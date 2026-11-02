---
section: Migration notes
---
- **File storage (Rust API, #2179):** a custom File object lends its
  `FileStorage` from `file_storage()` and `file_storage_mut()` instead of the
  `_internal` names, with the same bodies; callers rename the same way
  ([details](docs/rust-api.md#capabilities)).
