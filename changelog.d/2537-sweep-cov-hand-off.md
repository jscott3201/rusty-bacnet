---
section: Changed
---
- **Wire:** An object a later waiting sweep pass also reports may be left to
  that pass, once at most, so its COV report goes within twice
  `cov_retry_timeout_ms` of the commit that wanted it; an earlier pass's
  fanout already reading the object can still report a later state (#2537).
