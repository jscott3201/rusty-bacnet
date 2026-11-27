---
section: Added
---
- `OutboundTransactionCoordinator::route_through` records where a routed
  lease's attempt went, so only an answer from that station completes it, and
  `off_path_answers` counts the rest (#2831).
