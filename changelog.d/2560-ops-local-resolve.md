---
section: Changed
---
- **Breaking (Rust API):** a `bacnet_ops` handle made from a bare instance
  no address claims asks this network alone, not every network, unless
  `Client::with_resolve_scope` widens it, and a stalled handle asks again on
  its device's last network (#2560).
