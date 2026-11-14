---
section: Migration notes
---
- Drop `AsBip` bounds and imports and call `TransportPort::as_bip`; a transport that wrapped another overrides `data_link`, `bip_transport` and `bip6_transport` to forward to it (#2560).
