---
section: Changed
---
- **Breaking (Rust API):** WritePropertyMultiple moves to
  `bacnet-device-core` (`write::write_property_multiple`), whose `Responder`
  now answers ReadPropertyMultiple and WritePropertyMultiple within `Limits`'
  new RPM budget. The legacy RPM and WPM handlers are deleted (#2178).
