---
section: Changed
---
- **Wire:** The fault-detection, Schedule and operation tasks' COV fanout
  follows each pass's event notifications in pass order, off their loops and
  within `cov_retry_timeout_ms` of the commit, as the sweep's does; a
  destination that holds its lane delays none of their later passes (#2537).
