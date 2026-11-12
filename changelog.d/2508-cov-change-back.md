---
section: Fixed
---
- **Wire:** `BACnetServer` reports a COV change back to the value a
  subscriber last heard made while the previous notification was still to
  go (it sends both), and looks again at a change measured from a
  notification that then went unsent (#2508).
