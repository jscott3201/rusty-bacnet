---
section: Changed
---
- **Rust API:** `bacnet-device-core` decides what a COV-multiple fanout
  reports (`cov::multiple_report`), and `bacnet-server` sends what it
  returns. The split, its inputs and `cov::multiple_items` and
  `cov::multiple_reads` are private to the crate again (#2320).
