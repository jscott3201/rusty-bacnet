---
section: Migration notes
---
- **Property list (Rust API, #2197):** drop `fn property_list` and
  `fn required_properties` from custom objects; `.property_list()
  .contains(&p)` becomes `has_property(&rows, p)`. The Device's old list
  included Property_List, and `has_property` still answers true for it.
  `property_list_from_metadata` and `required_properties_from_metadata`
  become `property_list(&rows)` and `required_properties(&rows)`. The new
  list omits Object_Identifier, Object_Name and Object_Type; use the rows
  for the old set.
