---
section: Fixed
---
- BACnet/SC: primary restore dials and Connect handshakes no longer block
  traffic or heartbeat processing on the active failover hub. Stop, drop and
  loss of that connection cancel the pending restore; completed candidates
  cannot replace a connection that has started shutting down (#1575).
