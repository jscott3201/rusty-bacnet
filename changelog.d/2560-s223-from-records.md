---
section: Changed
---
- **Breaking (Rust API), Wire:** the Standard 223 export derives from the inventory's records and
  reads in batches (about N/8 + 2 requests, not N + 2). An object the device fails is skipped with
  `no readable object-name`, and a list read element by element that fails is an error (#2560).
