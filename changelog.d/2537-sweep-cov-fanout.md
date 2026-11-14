---
section: Changed
---
- **Wire:** The intrinsic-reporting sweep no longer waits for a pass's event
  notifications before its next pass: each pass's COV fanout waits for them
  behind earlier passes', up to `cov_retry_timeout_ms`, and past 64 waiting
  passes goes at once, counted in `cov_wait_expired` (#2537).
