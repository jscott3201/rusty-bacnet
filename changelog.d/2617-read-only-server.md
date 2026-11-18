---
section: Added
---
- **Rust API:** `bacnet_endpoint::session::read_only_server()` is public: the
  read-only server builder, no peer writing, creating or deleting and time
  synchronization off, that a session with a server role runs without
  `with_server`, as the Python endpoints do without `mutation_policy` (#2617).
