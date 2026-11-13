---
section: Changed
---
- **Wire:** A change's COV notifications wait, capped at
  `cov_retry_timeout_ms`, until its event notifications are handed to the
  network, a confirmed one's first send included; they used to wait only
  until its worker had started (#2537).
