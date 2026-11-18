---
section: Added
---
- A client-only endpoint session keeping a database for source Audit changes
  it with `EndpointSession::write_local`, `set_present_value_local` and
  `set_reliability_local`, saved and audited. A session with a server role
  refuses these; `server()`'s calls of those names make its changes
  (#2468, #2617).
