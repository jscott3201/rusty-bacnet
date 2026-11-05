---
section: Added
---
- `bacnet-device-core`'s `clock` module gives firmware a Device clock that
  TimeSynchronization and UTCTimeSynchronization set: the Device serves
  Local_Date and Local_Time from it, and Notification Class recipient
  windows open once it is synchronized, failing closed before (#2321).
