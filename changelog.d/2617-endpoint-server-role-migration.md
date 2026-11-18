---
section: Migration notes
---
- **Endpoint server role (Rust API, #2617):** compose a server-role session
  with `device(identity)`, or use `role(SessionRole::ClientOnly)` for a node
  that only asks. For writes give `with_server` a builder whose `ServerPolicy`
  decides them; move a reinitialize handler and password and a read work
  limit (as `read_property_multiple_budget`) onto it ([table](docs/rust-api.md#the-narrow-server-role-is-gone)).
