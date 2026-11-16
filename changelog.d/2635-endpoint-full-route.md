---
section: Fixed
---
- A burst of traffic no longer ends an endpoint session: what a full queue
  can't take is dropped and counted by class, and each queue keeps a reserve
  for confirmed requests and answers to the endpoint's own requests (#2635).
