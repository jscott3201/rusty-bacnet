---
section: Fixed
---
- A Schedule's target writes and the server's background passes (Binary
  Lighting Output operations, Access Door relocks, the fault pass) still get
  their event-state transition at the next sweep, not with the write (#2530,
  #2535).
