---
section: Migration notes
---
- **Discovery scope (Rust API, #2588):** `who_is(range)` is
  `who_is(Broadcast::Global, range)`, `who_is_network(n, range)` is
  `who_is(Broadcast::Network(n), range)`, `who_has(object, range)` is
  `who_has(Broadcast::Global, object, range)`, and `who_has_replies` takes
  the scope first. A Who-Is or Who-Has encoded by hand for
  `broadcast_unconfirmed` is `Broadcast::Local`.
