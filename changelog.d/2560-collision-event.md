---
section: Changed
---
- **Breaking (Rust API):** `DeviceCollisionEvent` swaps `retained` and
  `incoming` for `instance` and `claimants`, and comes again when the contest
  ends (`is_resolved()`); a second claimant's I-Am is announced as
  `Discovered` (#2560).
