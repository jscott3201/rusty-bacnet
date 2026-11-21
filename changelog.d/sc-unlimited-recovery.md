---
section: Added
---
- **SC reconnect (Rust and Python API):** `max_retries: None`
  (`ScReconnect(max_retries=None)` in Python) keeps recovering through a long
  hub outage, trying both hubs each round; it needs an initial delay of at
  least one second.
