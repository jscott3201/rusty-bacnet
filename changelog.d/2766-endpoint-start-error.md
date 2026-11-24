---
section: Changed
---
- **Breaking (Rust API):** The endpoint builders' `build_session*`, `build_*_session` and `connect_hub_session*`, and `EndpointSession::start`, return `StartError`, which hands back the object database: a builder's as given, and a start's once a refusal ends the session (#2766).
