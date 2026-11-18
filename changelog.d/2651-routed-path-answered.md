---
section: Changed
---
- **Wire:** A routed request the device answered after a retry or in segments
  frees its router and network at once instead of holding them for the APDU
  timeout times the attempts; a stale reason-4 reject it could draw is counted
  in `BACnetClient::too_long_reject_drops()` (#2651).
