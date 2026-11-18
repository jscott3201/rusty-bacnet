---
section: Fixed
---
- A message-too-long reject for a segmented routed request now teaches the
  path's limit from the largest segment it sent, not a smaller final one
  (#2651).
