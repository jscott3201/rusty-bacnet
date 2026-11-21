---
section: Changed
---
- **Wire:** A B/IP device on a loopback `interface` whose socket is bound to `0.0.0.0` no longer broadcasts to 255.255.255.255, which the host routed onto the LAN: the send fails, counted, and a BBMD still forwards to its peers and foreign devices. Part of #2691.
