---
section: Changed
---
- **Breaking (Rust API):** `bacnet-ops` watches points by address:
  `Client::watch` streams `Update`s, each with one `State` holding its
  reason, for points from `Device::point`; the hub under it moves to
  `watch::hub`, and a device is given up as moved only when the move is
  certain (#2560).
