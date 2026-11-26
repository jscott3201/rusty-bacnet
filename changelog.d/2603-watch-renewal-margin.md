---
section: Changed
---
- A client's watch renews each subscription early enough for two requests
  with every retry on the client's own timers, which `BACnetClient` now
  reports (`apdu_timeout`, `apdu_retries`): 48 s before a five-minute
  lifetime ends at the default 6 s and three retries, was 60 s (#2603).
