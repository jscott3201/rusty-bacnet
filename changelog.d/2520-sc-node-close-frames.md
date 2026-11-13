---
section: Changed
---
- **Wire:** a BACnet/SC node closes the WebSockets it ends: 1000 on `stop()`
  (after the Disconnect-ACK, waited for up to a second), after any NAK or
  answered Disconnect-Request, no status when a peer stops answering or
  replies wrongly, and 1003 for a text frame on a direct link (#2520).
