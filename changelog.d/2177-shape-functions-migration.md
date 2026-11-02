---
section: Migration notes
---
- **Writability and shapes (Rust API, #2177):** call
  `property_metadata::is_writable_property(&object, p)`, `is_array_property`
  or `is_list_property` instead of the trait methods. An override moves onto
  the rows: the write capability, or a vendor array or list with
  `with_value_type`. `decode_write_property_value` takes the property's
  `Option<ValueType>` in place of the list flag
  ([guide](docs/rust-api.md#migrating-custom-objects-to-016)).
