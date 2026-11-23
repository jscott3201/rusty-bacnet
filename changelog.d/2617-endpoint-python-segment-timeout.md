---
section: Added
---
- **Python API:** `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take a keyword-only
  `apdu_segment_timeout_ms` (default 5000), the Device's APDU_Segment_Timeout that both roles use
  while they segment, as `BACnetServer` does (#2617).
