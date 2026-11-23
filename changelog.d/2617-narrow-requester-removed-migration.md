---
section: Migration notes
---
- **Endpoint client (Rust API, #2617):** use `session.client()?`, and for
  each destination form and `data_attributes` the client's call in the
  [migration table](docs/rust-api.md#the-narrow-client-role-is-gone):
  `Station::routed`, `Station::routed_via_local_broadcast`,
  `client.with_data_attributes` (#2723).
