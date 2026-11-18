---
section: Changed
---
- `configure_routed_path_max_npdu` and `clear_routed_path_limit` no longer wait out the hold a routed request with no answer leaves on its path: the change applies at once, and the next request still waits the hold out, at the new limit (#2667).
