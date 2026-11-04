---
section: Changed
---
- **Rust API:** an endpoint's transaction coordinator matches a direct answer
  to a request routed to a network that has since become its own (new
  `CanonicalPeer::direct_alias`); this applies to every endpoint role, the
  server's notification transactions included (#2371).
