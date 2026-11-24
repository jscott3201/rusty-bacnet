---
section: Migration notes
---
- **Endpoint builders and start (Rust API, #2766).** `build_session`, `build_session_wrapped`, `build_loopback_session`, `build_hub_session`, `connect_hub_session(_wrapped)` and `EndpointSession::start` return `Result<_, StartError>`. `?` into `bacnet_types::error::Error` still works; elsewhere add `.map_err(Error::from)`, match on `error()`, or take `into_parts()` for the error and the database.
