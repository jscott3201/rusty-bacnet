---
section: Fixed
---
- **Wire:** Changing a Staging object's Target_References or
  Priority_For_Writing now relinquishes the priority slots the new
  configuration no longer writes, but only slots it still holds, as a
  Schedule does (#2289).
