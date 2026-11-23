---
section: Fixed
---
- Confirmed event notifications waiting for an invoke ID get one in the
  order they began to wait; one arriving behind others queues, and counts
  as a wait, even with an ID free. Once other work takes a freed ID ahead of
  the oldest, the next is kept for it, on an endpoint from its client too
  (#2563).
