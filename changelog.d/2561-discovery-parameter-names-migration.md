---
section: Migration notes
---
- **Discovery parameters (Python API, #2561):** rename `low_limit=`/`high_limit=` to `low=`/`high=` on `who_is` and the Who-Has calls, and `timeout_ms=3000` to `wait=3.0` (seconds, 0 to 3600) on `who_has_replies` and `discover_routers`. Pass them and `scope` by keyword: `who_has_replies(obj, 1, 9)` is `who_has_replies(obj, low=1, high=9)`.
