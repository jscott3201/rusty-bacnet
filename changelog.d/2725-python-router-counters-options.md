---
section: Added
---
- **Python API:** `BACnetRouter` takes the keyword-only router options
  `remote_table_updates`, `next_hop_rejects` and `control_policy`, and
  `await router.counters()` returns the router's counts by name, the
  isolation counts and wire-control decisions among them (#2725).
