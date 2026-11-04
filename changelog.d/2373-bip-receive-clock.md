---
section: Changed
---
- **Breaking (Rust API):** `bacnet_bip_core::node::Link::receive` and
  `Policy::admit_management` no longer take the time, so the B/IP transport
  reads no clock for an ordinary received datagram, about 90 fewer
  instructions per datagram (#2373).
