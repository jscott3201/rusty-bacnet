---
section: Migration notes
---
- **Custom objects (Rust API, #2181):** implement `property_metadata` with a
  row per served property, universal ones included. With
  `use bacnet_objects::property_metadata::{PropertyConformance as C,
  PropertyMetadata as M, PropertyWriteCapability as W};` a row is
  `M::new(PropertyIdentifier::OBJECT_NAME, C::RequiredRead, None, W::ReadOnly)`.
  `property_list()` is still required and must match the rows:
  `property_list_from_metadata` derives it, and #2197 tracks making the rows
  the only source.
