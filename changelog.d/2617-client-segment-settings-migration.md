---
section: Migration notes
---
- **Client segmentation (Rust API, #2617):** replace `segmented_response_accepted: false` with
  `segmentation: Segmentation::SEGMENTED_TRANSMIT` (or `NO_SEGMENTATION` to send no segments
  either), and the builders' `.segmented_response_accepted(..)` with `.segmentation(..)`. A struct
  literal names `segmentation` and `apdu_segment_timeout_ms`, or ends with
  `..ClientConfig::default()`.
