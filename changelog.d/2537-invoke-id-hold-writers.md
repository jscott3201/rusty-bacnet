---
section: Changed
---
- **Wire:** A write whose transition goes to a destination waiting for an
  invoke ID waits up to `cov_retry_timeout_ms` before its COV fanout, counted
  in `cov_wait_expired`: only then does `write_local` return, or a request
  give up its admission slot, after its reply (#2537).
