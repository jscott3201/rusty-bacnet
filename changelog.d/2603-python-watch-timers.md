---
section: Fixed
---
- **Python API:** a watch on a client from `endpoint.client()` renews early
  enough for the endpoint's `apdu_timeout_ms` and `apdu_retries`, not the
  default timers, so a long timeout no longer lets a subscription lapse
  (#2603).
