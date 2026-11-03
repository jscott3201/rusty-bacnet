---
section: Migration notes
---
- **bip-core node actions (Rust API, #2307):** a `match` on `node::Action` handles `ReleasePrevious { exchange, to, entry }`: send `Action::message()` to `to` and report it with `Node::exchange_sent` or `Node::exchange_failed`, as for `Register`. `NodePort` does this already.
