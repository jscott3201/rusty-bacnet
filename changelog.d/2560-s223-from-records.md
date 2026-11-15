---
section: Changed
---
- **Breaking (Rust API), Wire:** the Standard 223 export derives from the inventory's records and
  reads in batches (about N/8 + 2 requests, not N + 2); `export::s223`'s `Inventory`, `Entry`,
  `InventoryOptions`, `Detail`, `read_inventory` and `WritePriority` are replaced (#2560).
