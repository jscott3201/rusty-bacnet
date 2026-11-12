---
section: Changed
---
- **Wire:** the BACnet/SC hub sends a close frame with a status when it ends a
  connection on purpose: 1001 on a graceful shutdown, else 1000, also after
  refusing a Connect-Request, which it used to drop silently. A malformed one
  gets a close with no status; the direct acceptor sends 1000 too (#2463).
