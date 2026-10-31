---
section: Added
---
- **Breaking (Rust API):** New `bacnet-device-core` crate, a sans-IO request
  core over an `ObjectDatabase` (`no_std` with an allocator, or hosted).
  `bacnet-server` now answers ReadProperty, Who-Is and Who-Has through it,
  and firmware can run them with its `Responder` (#2178).
