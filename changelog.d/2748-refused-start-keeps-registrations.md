---
section: Fixed
---
- **Python API:** A `BACnetServer.start()` refused before the objects reach the database (a bad `registered_network_port`, a refused SC dial) keeps its registrations; `start()` on a running server raises (#2748).
