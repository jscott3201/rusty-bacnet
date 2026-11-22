---
section: Changed
---
- **Breaking (Rust API):** `configure_routed_path_max_npdu` and `clear_routed_path_limit` are synchronous and apply at once: they wait neither out the hold a routed request with no answer leaves nor for a request on the path; one waiting out the hold goes sized to the new limit (#2667, #2683).
