---
section: Migration notes
---
- **Watch points and following writes (Rust API, #2795):** write
  `Point { device: Binding::At(address), .. }` (or `address.into()`);
  `Device::point` picks `Binding::Follows` for a following handle. Read where
  a point is watched from `Update::at` or `Lease::at`; a following point
  reports moves in `Update::moved`, not `Why::Moved`. Match
  `WatchError::{UnknownDevice, Ambiguous}`, `Why::Unconfirmed` and
  `OpsError::NotConfirmed`: read through the handle, then write again.
