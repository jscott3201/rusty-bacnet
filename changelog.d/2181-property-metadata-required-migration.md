---
section: Migration notes
---
- **Custom objects (Rust API, #2181):** implement `property_metadata` with one
  row per property served, the universal ones included, such as
  `Cow::Borrowed(&[PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead,
  None, ReadOnly), …, PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite,
  None, Always), PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None,
  ReadOnly)])`. Writability and required properties derive from the rows; the
  trait docs show a full object.
