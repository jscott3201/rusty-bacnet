---
section: Added
---
- A host can change the Device clock's zone at run time, for daylight saving
  or a corrected UTC offset, without losing a synchronization:
  `BACnetServer::set_clock_zone` and `clock_zone`, and in the device core
  `SyncedClock::set_zone`, `Responder::set_zone` and `Reporter::set_zone`
  (#2395).
