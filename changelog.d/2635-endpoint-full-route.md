---
section: Fixed
---
- A burst of traffic no longer ends an endpoint session: what a full queue
  can't take is dropped and counted by class, and unconfirmed requests leave
  a quarter of each queue for confirmed requests and answers (#2635).
