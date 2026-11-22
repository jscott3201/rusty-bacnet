---
section: Changed
---
- **Breaking (Rust API):** `configure_routed_path_max_npdu` and `clear_routed_path_limit` are synchronous and apply at once, ahead of a request waiting out a routed path's hold, which then goes sized to the new limit; the hold itself is unchanged (#2683).
