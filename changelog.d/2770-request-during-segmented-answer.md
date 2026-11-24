---
section: Fixed
---
- **Wire:** A different request under the invoke ID of a segmented answer still being sent gets
  one Abort and is not executed; the answer ends and the invoke ID is free. The same request
  sent again is still dropped as a duplicate, so the answer goes on (#2770).
