---
section: Migration notes
---
- **Server builders (Rust API, #2752).** Every server `build()` and `BACnetServer::start`, `start_with_clock` and `start_clockless` return `Result<_, StartError>`. `?` into `bacnet_types::error::Error` still works; match on `error()`, or take `into_parts()` for the error and the database to build again.
