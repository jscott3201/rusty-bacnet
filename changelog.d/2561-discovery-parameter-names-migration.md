---
section: Migration notes
---
- **Discovery parameters (Python API, #2561):** rename `low_limit=`/`high_limit=` to `low=`/`high=` on `who_is` and the Who-Has calls, and `timeout_ms=3000` to `wait=3.0` (seconds) on `who_has_replies` and `discover_routers`. `apdu_timeout_ms` and the other link timeouts are unchanged.
