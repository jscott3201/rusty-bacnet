---
section: Migration notes
---
- **Device identity (Rust API and wire, #2617):** `DeviceIdentity::new` now segments both ways. A
  device that must not (a `bacnet-device-core` `Responder`, a constrained MS/TP link, a test pinned
  to the old I-Am) calls `.with_segmentation(Segmentation::NO_SEGMENTATION)`; a Python endpoint takes
  `segmentation=Segmentation.NO_SEGMENTATION`. An endpoint session no longer refuses a segmenting
  identity or Device.
