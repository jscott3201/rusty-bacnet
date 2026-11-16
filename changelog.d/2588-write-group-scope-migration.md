---
section: Migration notes
---
- **WriteGroup (Rust API, #2588):**
  `write_group(&WriteGroupDestination::LocalBroadcast, &req)` is
  `write_group(Broadcast::Local, &req)`; `RemoteBroadcast(n)` is
  `Broadcast::Network(n)` and `GlobalBroadcast` is `Broadcast::Global`.
  `write_group(&WriteGroupDestination::Device(mac), &req)` is
  `write_group_directed(&mac, &req)`.
