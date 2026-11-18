---
section: Added
---
- **Python API:** `BACnetClient` sets and clears a routed path's size limit, lists the limits with their source (`routed_path_limits()`, also in Rust) and reads both too-long reject counters; the CLI takes `--routed-path-limit ROUTER/NETWORK=MAX_NPDU` and reports routed paths on stderr (#2667).
