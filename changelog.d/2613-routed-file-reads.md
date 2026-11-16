---
section: Added
---
- **Python API:** `atomic_read_file` and `atomic_write_file` take a
  `RoutedTarget`, on a standalone client and an endpoint's, with the routing
  rules of the other typed calls; segmented answers and error detail are as
  for a local read (#2613).
