---
section: Changed
---
- **Breaking (Python API):** every discovery call takes `low=`, `high=` and, where it waits, `wait=` in seconds: `who_is`, `who_has_by_id`, `who_has_by_name`, `who_has_replies` and `discover_routers` drop `low_limit`, `high_limit` and `timeout_ms` (#2561).
