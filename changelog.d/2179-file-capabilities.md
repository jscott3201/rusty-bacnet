---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject::file_storage_internal` and
  `file_storage_internal_mut` are renamed `file_storage` and
  `file_storage_mut`; the hidden File configuration accessors drop
  `_internal` the same way (#2179).
