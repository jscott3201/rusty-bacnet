---
section: Changed
---
- **Breaking (Rust API):** each property metadata row now carries the
  property's datatype and shape, from a standard table in `bacnet-types`, and
  the server's list, ReadRange, NULL-write and local-reference handling reads
  it from the rows (#2177).
