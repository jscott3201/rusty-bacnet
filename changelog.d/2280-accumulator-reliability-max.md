---
section: Fixed
---
- **Wire:** An Accumulator's Reliability accepts a write while it is out of
  service and gets its prior value back on the return, and a Max_Pres_Value
  write below the current Present_Value is now refused instead of leaving the
  count out of range (#2280).
