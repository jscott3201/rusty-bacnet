---
section: Changed
---
- **Wire:** An unbound Device recipient holds its lane while it is looked
  for, in commit order. A write that made one waits for its I-Am, up to
  `cov_retry_timeout_ms`, before its COV fanout: `write_local` and
  `set_present_value_local` return, and a request frees its admission slot,
  only then (#2537).
