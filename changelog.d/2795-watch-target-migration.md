---
section: Migration notes
---
- **Watch points (Rust API, #2795):** write `Point { device:
  Target::At(address), .. }` (or `address.into()`) where you wrote the
  address; `Device::point` already picks `Target::Follows` for a following
  handle. Read the address a point is watched at from `Update::at` (or
  `Lease::at`), not `point.device`. A following point no longer ends as
  `Why::Moved`: watch `Update::moved`. Match the new `WatchError` variants
  `UnknownDevice` and `Ambiguous`.
