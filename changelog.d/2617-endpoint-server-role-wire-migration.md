---
section: Migration notes
---
- **Endpoint wire behaviour (Rust API, #2617):** an endpoint session with a
  server role is now discoverable: it answers a global Who-Is with its I-Am
  and serves every read. A node that only asks stays silent as a
  `SessionRole::ClientOnly` session. A tool that read Protocol_Services_Supported
  to learn what it may write must try the write: the read-only server denies
  writes it lists.
