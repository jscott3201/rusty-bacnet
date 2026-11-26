---
section: Added
---
- **Python API:** `Device.watch` on a handle made from an instance follows
  the device to a new address: `WatchUpdate.address` says where a point is
  watched now, with `WatchUpdate.device`, `WatchUpdate.moved` (a new
  `WatchMove`) and `Watch.follows` (#2795).
