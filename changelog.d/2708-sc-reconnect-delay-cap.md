---
section: Changed
---
- **Breaking (Rust and Python API):** a BACnet/SC reconnect `max_delay_ms` may
  be at most 600,000 ms (`ScReconnectConfig::MAX_DELAY_MS`, the most Annex
  AB.6.1 lets the reconnect wait grow to) instead of 24 hours; validation
  refuses a larger value (#2708).
