---
section: Added
---
- `bacnet_ops::DeviceAddress` and `Device` handles: pinned to an address,
  or following an instance on one network or any, moving only once its old
  address has gone quiet; an ambiguous instance is refused naming every
  address. `Client::duplicates` and `events` report duplicates and moves
  (#2560).
