---
section: Added
---
- `TransportPort::link_returns` counts each time a transport's link comes
  back: BACnet/SC hub reconnects and failovers, and B/IP and B/IPv6 foreign
  devices registering again after a loss. Wrappers must delegate it (#2572).
