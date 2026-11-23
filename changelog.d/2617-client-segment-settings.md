---
section: Added
---
- **Breaking (Rust API):** `ClientConfig` gains `segmented_requests` (default on; off, a request that
  needs segments fails with `SegmentationError::LocalCannotTransmit` before anything is sent) and
  `apdu_segment_timeout_ms` (default: the APDU timeout) (#2617).
