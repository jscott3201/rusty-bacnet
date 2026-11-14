---
section: Migration notes
---
- A transport that wraps another and implemented `AsBip` now overrides `TransportPort::bip_transport` (and `data_link`) to forward to the transport it wraps, and drops its `AsBip` impl (#2560).
