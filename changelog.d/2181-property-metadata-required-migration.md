---
section: Migration notes
---
- **Custom objects (Rust API, #2181):** implement `property_metadata` with a
  row per served property, universal ones included. With
  `use bacnet_objects::property_metadata::{PropertyConformance as C,
  PropertyMetadata as M, PropertyWriteCapability as W};` a row is
  `M::new(PropertyIdentifier::OBJECT_NAME, C::RequiredRead, None, W::ReadOnly)`.
  The rows are the only list an object writes (#2197).
