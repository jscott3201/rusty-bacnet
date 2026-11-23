---
section: Changed
---
- **Breaking (wire):** `DeviceIdentity::new` defaults to SEGMENTED_BOTH, so a Rust server or endpoint
  built from it, and a Python endpoint, announces and serves segmentation both ways;
  `bacnet-device-core`'s `Responder` needs an identity set to NO_SEGMENTATION (#2617).
