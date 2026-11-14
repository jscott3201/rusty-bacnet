---
section: Added
---
- `bacnet_ops::DeviceAddress` and `Device` handles, pinned to an address or
  following an instance: moving when another device takes its address, or
  after a timeout once only another address answered the Who-Is it sent
  (one per `QUIET`). `Client::duplicates` and `events` report both
  (#2560).
