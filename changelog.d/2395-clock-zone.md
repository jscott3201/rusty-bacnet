---
section: Added
---
- A host can set the Device clock's zone, for daylight saving or a corrected
  UTC offset, without losing a synchronization: the builders' `clock_zone`,
  `BACnetServer::set_clock_zone` and `clock_zone`, and in the device core
  `set_zone` on `SyncedClock`, `Responder` and `Reporter` (#2395).
