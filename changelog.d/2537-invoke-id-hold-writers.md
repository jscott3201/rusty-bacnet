---
section: Changed
---
- **Wire:** A write whose transition goes to a destination whose lane waits
  for an invoke ID waits up to `cov_retry_timeout_ms` before its COV fanout,
  counted: `write_local` returns, and a request gives up its admission slot
  after its reply, only then (#2537).
