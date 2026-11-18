---
section: Migration notes
---
- **Endpoint full server (Rust API, #2617):** compose the session with
  `device(identity)` before `with_server`. Leave the builder's identity unset,
  or set the session's, and give a database whose Device declares the
  session's largest APDU and APDU timing, or the object model's defaults,
  which the server replaces. Set the timing on `SessionConfig`.
