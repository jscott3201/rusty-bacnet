---
section: Added
---
- `bacnet_ops::DeviceAddress` and `Device` handles: pinned to an address or
  following an instance that moves, an ambiguous instance refused by name;
  `Client::discover` flags duplicates, and `Client::duplicates`,
  `Client::events` and `Device::moved_to` report them and moves (#2560).
