---
section: Added
---
- An endpoint session with a source Audit Reporter audits its full client's reads and writes,
  each request owned by the session once admitted, so `EndpointSession::full_client` and Python's
  `endpoint.client()` work on audited endpoints (#2617).
