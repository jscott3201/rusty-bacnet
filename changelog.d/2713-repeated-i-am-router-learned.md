---
section: Fixed
---
- With duplicate suppression on, a repeated I-Am-Router-To-Network is learned
  on its route's own port, so a station that sent a router's list first there
  no longer keeps the router's copy out. A first sender on another port holds
  the route until the router announces again or it ages out (#2713).
