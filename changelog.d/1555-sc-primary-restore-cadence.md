---
section: Fixed
---
- BACnet/SC: after a failed attempt to return to the primary hub, the next
  attempt waits a full restore interval, and a failover after a long spell on
  the primary no longer sets off a burst of attempts (#1555).
