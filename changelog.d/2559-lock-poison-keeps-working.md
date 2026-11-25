---
section: Changed
---
- After a panic poisoned its lock, the Device clock keeps its clock properties
  and takes zone writes and time synchronization instead of refusing them,
  and the Python server no longer raises `RuntimeError` "lock poisoned"; the
  invoke-ID leases still fail closed (#2559).
