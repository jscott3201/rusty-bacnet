---
section: Changed
---
- **Wire:** the BACnet/SC hub, and a node's direct-connection acceptor, close
  a connection they end on purpose with WebSocket status 1000, so a peer reads
  an orderly end; a close over the peer's own fault is unchanged (#2463).
