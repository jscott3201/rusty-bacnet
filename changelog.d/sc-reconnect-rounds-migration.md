---
section: Migration notes
---
- **SC reconnect (Rust, Python, TUI/CLI profiles):** Wrap
  `ScReconnectConfig.max_retries` in `Some(..)`. A count of 0 (Python
  `max_retries=0`, profile `reconnect_max_retries = 0`) now means no
  recovery, where it allowed one failover dial: use 1 to keep that turn. A
  raw `ScTransport` with no connector sends one Connect-Request on a lost
  socket and closes it when that fails, a duplicate-VMAC NAK included, so
  add a connector to keep recovering.
