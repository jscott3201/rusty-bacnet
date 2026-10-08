---
section: Migration notes
---
- **Tags (Rust API, #1583):** Replace `TagValue::Primitive(value)` with `value`;
  `BACnetNameValue.value` is now `Option<PropertyValue>`. Use separate named
  tags for a Date and a Time. Old snapshots containing a combined pair fail
  to load and remain intact; correct them explicitly before reconstruction.
