---
section: Migration notes
---
- **who_has_replies (Rust API, #2596):** `BACnetClient::who_has_replies` can
  now return more than one row per device and object: one heard directly
  and through a router, through two routers, or from two devices sharing an
  instance; the list stops at 4096. Dedupe by device and object for one row
  each.
