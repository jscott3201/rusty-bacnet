---
section: Migration notes
---
- **Property list (Rust API, #2197):** delete `fn property_list` and
  `fn required_properties` from custom objects. `object.property_list()
  .contains(&p)` becomes `has_property(&object.property_metadata(), p)`;
  `property_list(&rows)` (the served list, without the identity properties)
  and `required_properties(&rows)` are iterators, and
  `read_property_list(rows, index)` answers a Property_List read.
