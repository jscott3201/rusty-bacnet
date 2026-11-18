---
section: Migration notes
---
- **Endpoint server role (Rust API, #2617):** compose a server-role session
  with `device(identity)`, or make a node that only asks a
  `SessionRole::ClientOnly` session. In place of `with_device_writes`, give
  `with_server` a builder whose `ServerPolicy` decides writes; move a
  reinitialize handler and password and a `read_work_limit` (as
  `read_property_multiple_budget`) onto that builder
  ([table](docs/rust-api.md#the-narrow-server-role-is-gone)).
