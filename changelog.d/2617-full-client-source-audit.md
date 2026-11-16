---
section: Added
---
- An endpoint session with a source Audit Reporter audits its full client's reads and writes, each
  owned by the session once admitted; past 64 at once a request waits for a slot.
  `EndpointSession::full_client` and Python's `endpoint.client()` work on audited endpoints (#2617).
