---
section: Migration notes
---
- **Router application local scope (Rust and Python API, #2601):**
  `Broadcast::answered_from(source, own)` takes the client's local networks
  now: pass `&client.local_networks()`. A new `TransportPort` method,
  `attached_networks`, names them, and a wrapper must forward it. In Python a
  router client's `resolve_scope` reads `"local"` instead of None, and its
  default `discover()` no longer raises, so a network number passed only to
  avoid that error can go.
