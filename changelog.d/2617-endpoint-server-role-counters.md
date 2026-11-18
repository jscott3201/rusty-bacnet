---
section: Changed
---
- **Breaking (Rust API):** An endpoint session with a server role counts the
  requests its server is handed in `server_seat_counters`, so
  `PolicyCountersSnapshot`'s `confirmed_full_drops` and
  `unconfirmed_full_drops` stay 0 there (#2617).
