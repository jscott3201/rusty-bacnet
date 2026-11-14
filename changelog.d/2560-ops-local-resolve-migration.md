---
section: Migration notes
---
- **ops device lookup (Rust API, #2560):** name a device behind a router
  with its network (`5:1234`), or set `with_resolve_scope(ResolveScope::Global)`
  (or a network) on the client, or `.resolve_scope(..)` on its connect
  builder; a miss is `OpsError::UnknownDevice`, whose new `asked` field says
  where the Who-Is went.
