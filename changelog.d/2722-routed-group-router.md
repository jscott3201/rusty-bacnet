---
section: Changed
---
- **Wire:** a client's routed confirmed request whose router is the link's
  broadcast or a group address is refused before anything is sent, in Rust
  and Python alike; name the router's own address, or reach the network by
  `Station::routed_via_local_broadcast` (#2722).
