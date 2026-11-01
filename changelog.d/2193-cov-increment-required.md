---
section: Fixed
---
- **Wire:** COV_Increment (and Pulse Converter's COV_Period) is now required
  of every object that reports COV, as its table says, so ReadPropertyMultiple
  REQUIRED returns it and the PICS no longer marks it optional (#2193).
