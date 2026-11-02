---
section: Fixed
---
- **Wire:** A whole-array write with no elements, or such an array among
  CreateObject's initial values, reaches the object instead of being refused
  as undecodable; the object takes it or answers by its own rules, so a
  read-only array refuses access (#2305).
