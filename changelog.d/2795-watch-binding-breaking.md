---
section: Changed
---
- **Breaking (Rust API):** a watch `Point` names its device by a
  `watch::Binding`, `Update` gains `at`, `instance` and `moved`, and a write
  through a following handle not confirmed where it holds the device fails
  with the new `OpsError::NotConfirmed`, sending nothing (#2795).
