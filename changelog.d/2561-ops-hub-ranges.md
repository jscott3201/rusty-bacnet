---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops` hubs on one client never share a range of subscriber process identifiers: `Hub::new` takes the lowest free range of 256 and returns `WatchError` (`Hubs` when all are held); `Hub::stream` and a cancel-safe `Watch::next` are new (#2561).
