---
section: Changed
---
- **Breaking (Rust API):** `DiscoveredDevice::last_announced` is when the
  row's address last sent an I-Am, which answers to requests don't move;
  `Error::UnknownDevice` names a device the client can't place (#2560).
