---
section: Changed
---
- **Wire:** COV-multiple contexts due at once go out in key order from a
  start that moves per fanout, where the server used a hash order that
  varied per run. Under a spent notification budget the contexts kept back
  therefore vary, as before (#2320).
