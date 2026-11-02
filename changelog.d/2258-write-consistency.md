---
section: Fixed
---
- **Wire:** Program_State is read-only: network and local writes are refused
  and the PICS lists it read-only. Objects' own writes now refuse an index on a
  non-array, change nothing when refused, and treat a NULL Description alike;
  Staging ends a simulated Reliability with Out_Of_Service (#2258).
