---
section: Changed
---
- **Breaking (Rust API):** a routed confirmed request that its router
  refuses at the network layer (no route, an addressing error, other) ends
  at once as the new `Error::NetworkRejected`, naming the router, the
  network and the reason, instead of running to its timeout (#2724).
