---
section: Migration notes
---
- **Station forms (Rust API, #2723):** `Station` has a fourth variant,
  `RoutedViaLocalBroadcast { network, mac, one_station }`; a `match` over every
  `Station` form adds an arm for it.
