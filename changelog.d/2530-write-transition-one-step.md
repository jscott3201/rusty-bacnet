---
section: Fixed
---
- **Wire:** a write and the event-state transition it causes commit
  together, so ReadProperty and ordinary COV notifications never show IN_ALARM
  without FAULT after a fault clears, and a read after the write sees the new
  Event_State; a peer's EventNotification may now precede its SimpleACK (#2530).
