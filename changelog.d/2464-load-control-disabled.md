---
section: Fixed
---
- **Wire:** While a Load Control's Enable is FALSE, a Start_Time or
  Shed_Duration write is acknowledged but not kept, and Enable TRUE starts
  nothing until the next Start_Time write (#2464).
