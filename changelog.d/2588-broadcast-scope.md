---
section: Changed
---
- **Breaking (Rust API):** `who_is` and `who_has` take a `Broadcast` scope
  (`Local`, `Network(n)` or `Global`) and `who_is_network` is gone; new are
  `who_has_directed`, `who_am_i` and `who_am_i_directed`. A network
  broadcast to 0 or 65535 is refused before anything is sent (#2588).
