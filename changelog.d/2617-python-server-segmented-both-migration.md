---
section: Migration notes
---
- **Python server segmentation (wire, #2617):** `BACnetServer()` now declares and does
  SEGMENTED_BOTH. Pass `segmentation_supported=Segmentation.NO_SEGMENTATION` to keep every request
  and answer to one APDU. With the default, `apdu_segment_timeout_ms=0` now raises ValueError.
