---
section: Changed
---
- **Wire:** a BACnet/SC node closes the WebSockets it ends: 1000 on `stop()`,
  after answering a Disconnect-Request and on leaving the failover hub, no
  status when a hub or peer stops answering, and 1003 for a text frame on a
  direct link, which the direct acceptor used to ignore (#2520).
