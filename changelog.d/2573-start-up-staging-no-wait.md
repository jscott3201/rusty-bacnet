---
section: Fixed
---
- A server's `start()` no longer waits for the event notifications of
  Staging plans run at start-up: a silent Device recipient cost it up to
  the APDU timeout each; they now go out in the background, and their COV
  fanout follows them there (#2573).
