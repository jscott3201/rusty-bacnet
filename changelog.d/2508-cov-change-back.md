---
section: Fixed
---
- **Wire:** a COV change back to the value a subscriber last heard, made
  before the previous notification went out, is reported: the device core's
  `Reporter` (endpoint COV, firmware) sends it in the queued one's place, and
  `BACnetServer` sends both (#2508).
