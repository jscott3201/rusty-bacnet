---
section: Changed
---
- **Breaking (Rust and Python API):** On a router's application, the local
  scope is each of the router's directly attached networks: a default
  Who-Is, Who-Has or `discover()`, and `resolve_scope="local"`, ask every
  one of them and no network beyond, where they used to raise (#2601).
