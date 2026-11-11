---
section: Changed
---
- **Breaking (Rust API):** the 223 export's `Inventory` in `bacnet-ops` holds the shared point-list types, reads everything a point list holds at `Detail::Full`, and skips an object that can't give its name, units or state count, in every front end (#2445).
