---
section: Migration notes
---
- **A station at a broadcast form (Rust and Python, #2709, #2738):** mark
  it one station: `Station::routed(router, network, &[0xFF]).one_station()`,
  or `RoutedTarget(network, b"\xff", router=..., one_station=True)`. A
  pattern on `Station::Remote` or `Station::Routed` listing every field adds
  `one_station` or `..`.
