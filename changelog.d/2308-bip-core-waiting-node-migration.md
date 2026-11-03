---
section: Migration notes
---
- **bip-core foreign BBMD (Rust API, #2308):** `node::Foreign::bbmd` is `Option<SocketAddrV4>`: `Foreign::new` sets `Some`, so wrap an address you assign in `Some`, and compare with `Some(peer)`. `Link::broadcast` refuses a waiting foreign node's broadcast with `SendRefusal::NoBbmd`; ask `Node::admit_broadcast` first, as before.
