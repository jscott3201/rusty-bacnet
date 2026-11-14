---
section: Changed
---
- **Wire:** A Device recipient with no binding holds its lane while the
  server looks for it, so its notifications stay in commit order, and a
  write that made one waits for the I-Am, up to `cov_retry_timeout_ms`, before
  its COV fanout; it used to go on at once (#2537).
