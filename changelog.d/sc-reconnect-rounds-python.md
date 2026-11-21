---
section: Changed
---
- **Python API:** `ScReconnect` reconnect rounds try the primary, then
  `sc_failover_hub`; `max_retries=0` now turns recovery off, and
  `max_retries=None` keeps recovering.
