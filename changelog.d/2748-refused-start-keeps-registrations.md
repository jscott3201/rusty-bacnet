---
section: Fixed
---
- **Python API:** A `BACnetServer.start()` refused before it hands the objects to the database (a hub that refuses the dial, a cancelled start) keeps its registrations; `add_*` and a second `start()` raise while one is in progress (#2748).
