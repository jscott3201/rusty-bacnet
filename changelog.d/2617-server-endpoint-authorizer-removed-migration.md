---
section: Migration notes
---
- **Device-write authorizer (Rust API, #2617):** where you built a
  `DeviceWriteAuthorizer`, decide the same writes in a `ServerPolicy`
  (`FnPolicy::new().mutation(..)`, which sees the same
  `MutationAuthorizationContext`) on the server builder you give the session's
  `with_server`; it is asked about every covered mutation of every object
  ([table](docs/rust-api.md#the-narrow-server-role-is-gone)).
