---
section: Changed
---
- **Breaking (Rust API):** `BACnetClient`'s by-instance `_from_device` and
  `_to_device` calls are gone: `resolve_device(instance)` returns the
  `Station` the discovery table names, which every typed call takes.
  `CovTarget` folds into `Station`, and the managed COV helpers take any
  station (#2588).
