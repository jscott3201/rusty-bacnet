---
section: Migration notes
---
- **Value types (Rust API, #2177):** `standard_array_property` and
  `standard_list_property` are gone; use
  `bacnet_types::datatype::standard_value_type(object_type, property)` and its
  `is_array()` or `is_list()`. Building a `PropertyMetadata` is unchanged; a
  custom object that overrode `is_array_property` or `is_list_property` for a
  vendor property can state it on the row with `with_value_type` instead.
