---
section: Migration notes
---
- **discover (Python API, #2561):** `discover(timeout_ms=3000, low_limit=a, high_limit=b)` is `discover(wait=3.0, low=a, high=b)`. Rows are `Discovered`: use `row.instance`, `row.address`, `row.network` and `row.entry` (the old `DiscoveredDevice`). `who_is_directed(address, ...)` is `who_is(scope=address, ...)`; an MS/TP station is `"mstp:5"`.
