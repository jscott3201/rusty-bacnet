---
section: Changed
---
- **Breaking (Rust API):** BACnet/SC recovery tries the primary, then the
  failover hub, in every reconnect round. `ScReconnectConfig.max_retries` is
  an `Option<u32>`, and `Some(0)` turns recovery off.
