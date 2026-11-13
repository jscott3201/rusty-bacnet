---
section: Fixed
---
- **Wire:** ReadProperty and ordinary COV no longer show IN_ALARM without
  FAULT after a fault clears, and a read after a write's response sees the new
  Event_State. The EventNotification may now arrive either side of the
  response; with Staging plans it is handed to the network before it (#2530).
