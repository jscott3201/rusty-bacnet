---
section: Changed
---
- **Breaking (Rust API):** `ConstructedElement` gains `BdtEntry` and `FdtEntry` (so `ALL` holds 30), and `split_elements` refuses a value of more than 10,000 elements, the bound the list codecs keep (#2690).
