---
section: Fixed
---
- **Wire:** A confirmed request over 64 KiB is now followed by the server's duplicate detection by a
  digest of its data, so a copy of it sent again while it executes is not run a second time and its
  repeated segments are acknowledged again (#2751).
