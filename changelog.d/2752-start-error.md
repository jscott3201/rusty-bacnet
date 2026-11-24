---
section: Changed
---
- **Breaking (Rust API):** A refused server `build()` or `BACnetServer::start` returns `StartError`, which hands back the database it took, with a registered Network Port it published put back, so the same objects can start again (#2752).
