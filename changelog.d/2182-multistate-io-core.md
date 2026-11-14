---
section: Changed
---
- The multi-state Input, Output and Value objects move onto the I/O core
  the analog and binary objects share, so all nine use one copy of it; the
  firmware image holding them is about 5 KB of flash smaller, and nothing
  the objects serve or save changes (#2182).
