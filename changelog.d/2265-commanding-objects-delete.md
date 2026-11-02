---
section: Fixed
---
- **Wire:** Deleting or removing a Staging object now relinquishes the
  priority slots its stage writes filled on its targets, but only slots it
  still holds, as a Schedule does (#2265).
