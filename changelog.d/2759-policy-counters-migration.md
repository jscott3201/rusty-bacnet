---
section: Migration notes
---
- **Endpoint policy counters (Rust API, #2759):**
  `EndpointSession::policy_counters()` is synchronous; drop the `.await`
  after it.
