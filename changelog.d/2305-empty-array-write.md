---
section: Fixed
---
- **Wire:** A whole-array write with no elements reaches the object instead of
  being refused as undecodable, so a read-only array answers that it can't be
  written and a writable one empties or refuses the size by its own rules
  (#2305).
