---
section: Fixed
---
- **Breaking (Rust API):** A generated PICS takes Protocol_Version,
  Protocol_Revision and its vendor block from the Device it describes, so it
  no longer claims revision 24 or prints stale details; `PicsConfig` loses
  those six fields (#2125, #2137).
