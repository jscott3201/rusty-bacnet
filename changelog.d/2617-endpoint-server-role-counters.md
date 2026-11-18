---
section: Changed
---
- **Breaking (Rust API):** An endpoint session with a server role counts the
  requests its server is handed in `server_seat_counters`, so
  `PolicyCountersSnapshot`'s `confirmed_full_drops`, `unconfirmed_full_drops`
  and `responder_declined` stay 0 there (#2617).
