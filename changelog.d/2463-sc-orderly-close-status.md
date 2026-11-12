---
section: Changed
---
- **Wire:** the BACnet/SC hub closes a connection it ends on purpose with a
  WebSocket status: 1001 on a graceful shutdown, else 1000, including after
  refusing a Connect-Request, which it used to drop with no close frame; the
  direct acceptor sends 1000 too, and faults are unchanged (#2463).
