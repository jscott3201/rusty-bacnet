---
section: Fixed
---
- A confirmed request waiting for its turn at a destination
  (`min_request_interval_ms`) ends at once when its client's tasks end (an
  endpoint session's stop or drop), as open requests do, and takes no turn
  (#2720).
