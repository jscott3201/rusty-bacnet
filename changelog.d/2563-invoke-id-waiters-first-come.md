---
section: Fixed
---
- Confirmed event notifications waiting for an invoke ID now get one in the
  order they began to wait, and once other server work has taken a freed ID
  ahead of the oldest, the next free one is kept for it, so no lane waits
  without bound (#2563).
