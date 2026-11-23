---
section: Changed
---
- **Breaking (Rust API):** `ClientConfig::segmentation` (default SEGMENTED_BOTH) replaces
  `segmented_response_accepted`: the client takes segmented answers and notifications only while it
  receives segments, and segments long requests only while it transmits. `apdu_segment_timeout_ms` is
  new (#2617).
