---
section: Fixed
---
- Confirmed event notifications waiting for an invoke ID get one in the
  order they began to wait, a later one queuing (and counting a wait) even
  with an ID free. Once other server work takes a freed ID first, the next
  the server can lease is kept for the oldest (#2563).
