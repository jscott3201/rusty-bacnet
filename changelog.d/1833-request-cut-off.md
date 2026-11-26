---
section: Added
---
- **Breaking (Rust API):** `Error::RequestCutOff { sent }` reports a confirmed
  request its client's stop ended before an answer came, and whether any of it
  may have gone out, where it was `Error::Encoding` text (#1833).
