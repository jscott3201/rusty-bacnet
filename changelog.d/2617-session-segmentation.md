---
section: Changed
---
- **Breaking (wire):** An endpoint session segments in both roles and both directions as its Device
  identity says, and its client now follows the identity's Segmentation_Supported, Max_Segments_Accepted
  and APDU_Segment_Timeout rather than settings of its own (#2617).
