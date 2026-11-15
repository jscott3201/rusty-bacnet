---
section: Changed
---
- **Breaking (Python API):** `discover(scope="local", low=None, high=None, wait=3.0)` returns one `Discovered` for each device that answered that call, no longer every `DiscoveredDevice` in the table, and `who_is_directed` is gone: `scope=` takes a station address (#2561).
