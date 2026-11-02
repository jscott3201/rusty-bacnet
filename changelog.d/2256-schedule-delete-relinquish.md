---
section: Fixed
---
- **Wire:** DeleteObject of a Schedule now relinquishes the priority slots it
  holds on its targets, as removing them from its reference list does, so its
  last command no longer stays behind (#2256).
