---
section: Fixed
---
- **Wire:** A different request under the invoke ID of a segmented answer still being sent gets
  one Abort and is not executed; once every segment is out and only the final Segment-ACK is due,
  it is served instead. The same request sent again is still dropped (#2770).
