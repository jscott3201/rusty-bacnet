---
section: Changed
---
- **Breaking (Rust API):** `write_group` takes a `Broadcast` scope, and
  `write_group_directed` sends to one station, behind a router too;
  `WriteGroupDestination` is gone (#2588).
