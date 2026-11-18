---
section: Added
---
- A client-only endpoint session with a database for source Audit changes it
  with `EndpointSession::write_local` (a stored property saved first, an
  Audit recipient write audited), `set_present_value_local` and
  `set_reliability_local`; a server-role session refuses them for
  `server()`'s (#2468, #2617).
