---
section: Added
---
- `bacnet-ops` watches take `WatchSettings` (lifetime, poll interval,
  subscribe or poll only) from the client's builder or per watch with
  `Client::watch_with`, sharing one hub per settings, and a `WatchControl`
  adds and removes points on a live watch; `Device::watch` is shorthand
  (#2603).
