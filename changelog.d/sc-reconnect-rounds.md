---
section: Changed
---
- **Breaking (Rust API):** BACnet/SC recovery tries the primary, then the
  failover hub, in every reconnect round, so the failover hub is dialled in
  round 1 instead of after every primary retry. `ScReconnectConfig.max_retries`
  is an `Option<u32>`, and `Some(0)` turns recovery off.
