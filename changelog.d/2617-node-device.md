---
section: Changed
---
- **Breaking (Rust API):** `EndpointSession::full_server` needs a session Device
  identity and serves the node's Device: both roles' services, the smaller
  largest APDU, and the session's APDU timeout and retries, refusing a builder
  or database that contradicts them (#2617).
