---
section: Added
---
- **Rust API:** `OutboundTransactionCoordinator::with_reserves` keeps part of
  the pool for each lease owner and `room` reads what one may lease now;
  `REQUESTER_RESERVE` and `NOTIFICATION_RESERVE` are a two-role node's.
  `ReserveError::Exhausted` now also means an owner is at its share (#2617).
