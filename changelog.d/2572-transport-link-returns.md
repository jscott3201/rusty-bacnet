---
section: Added
---
- `TransportPort::link_returns` counts each time a transport's link comes
  back after it was lost: BACnet/SC reports its hub reconnects and
  failovers, the other built-in transports none yet. Wrappers must delegate
  it (#2572).
