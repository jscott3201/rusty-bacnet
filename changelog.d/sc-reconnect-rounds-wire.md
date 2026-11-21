---
section: Changed
---
- **Wire:** after losing its hub, a BACnet/SC node dials the failover hub in
  the first reconnect round, right after a failed primary dial, instead of
  after every primary retry (about 50 minutes with the defaults).
