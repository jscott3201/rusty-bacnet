---
section: Fixed
---
- **Wire:** A reason 4 on a routed request of 228 NPDU octets or fewer (Table 6-1) is dropped and counted in `below_minimum_reject_drops()`; with no configured limit that is every one. A router limiting below 228 now makes requests time out: use `configure_routed_path_max_npdu` (#2665).
