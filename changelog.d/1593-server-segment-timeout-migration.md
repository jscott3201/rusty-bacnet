---
section: Migration notes
---
- **Segmentation (Rust/Python API, #1593):** Add `DeviceConfig::apdu_segment_timeout` to full literals and match it and segmentation mode to `ServerConfig`. Narrow endpoints require `NONE`. Python full servers add `segmentation_supported` and `apdu_segment_timeout_ms`; defaults remain unsegmented. See [server timing](docs/server-segmentation.md).
