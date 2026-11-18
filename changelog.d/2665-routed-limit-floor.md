---
section: Fixed
---
- **Wire:** A reason-4 reject landing on a routed request whose largest frame is 228 NPDU octets or fewer (Table 6-1's smallest) is now dropped and counted, not believed; stray ones no longer pin a path. A router limiting below 228 needs `configure_routed_path_max_npdu` (#2665).
