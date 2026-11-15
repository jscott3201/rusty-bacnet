---
section: Migration notes
---
- **Network number controls (Rust API, #2597):** match
  `NumberControl::WhatIs { broadcast }` where you matched
  `NumberControl::WhatIs`. An owner of the controls answers a broadcast
  question through `DelayedAnswer`, at its deadline, rather than at once.
