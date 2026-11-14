---
section: Changed
---
- **Wire:** The intrinsic-reporting sweep no longer waits for a pass's event
  notifications: each pass's COV fanout follows them in sweep order, within
  `cov_retry_timeout_ms` of its commit. A pass finding 64 waiting goes at
  once, counted in `cov_wait_expired` if its notifications still wait (#2537).
