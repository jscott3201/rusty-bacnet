---
section: Changed
---
- **Breaking (Python API):** every discovery call takes keyword-only `low=`, `high=`, `scope=` and, where it waits, `wait=` (0 to 3600 seconds): `who_is`, the Who-Has calls and `discover_routers` drop `low_limit`, `high_limit` and `timeout_ms` (#2561).
