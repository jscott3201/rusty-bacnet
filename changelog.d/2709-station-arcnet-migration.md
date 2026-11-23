---
section: Migration notes
---
- **ARCNET station 255 (Rust and Python, #2709):** mark it:
  `Station::routed(router, network, &[0xFF]).arcnet()`, or
  `RoutedTarget(network, b"\xff", router=..., arcnet=True)`. A pattern on
  `Station::Remote` or `Station::Routed` listing every field adds `arcnet`
  or `..`.
