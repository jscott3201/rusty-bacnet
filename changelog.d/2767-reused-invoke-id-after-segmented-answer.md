---
section: Fixed
---
- **Wire:** The full server ends a segmented answer as it takes the requester's final Segment-ACK
  or Abort, so a request that reuses the invoke ID right behind it is new: it is no longer
  discarded as a duplicate of the answered request, or acknowledged as a repeat of it (#2767).
