---
section: Added
---
- **Python API:** `BACnetClient(watch_lifetime=, watch_poll_interval=,
  watch_subscribe=)` set `Device.watch`'s defaults, which gains
  `subscribe=`; `Watch.add()` and `Watch.remove()` change a live watch,
  and watches with equal settings share their subscriptions (#2603).
