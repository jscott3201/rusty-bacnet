---
section: Added
---
- **Rust API:** `bacnet_endpoint::session::read_only_server()` is public: the
  read-only server builder, no peer writing, creating or deleting and time
  synchronization off, that the Python endpoints run when given no
  `mutation_policy`, for `EndpointSession::with_server` (#2617).
