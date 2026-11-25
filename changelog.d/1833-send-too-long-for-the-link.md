---
section: Fixed
---
- **Breaking (Rust API):** B/IP, MS/TP and Ethernet refuse an NPDU too long for
  the link as `Error::SendTooLong`, so a router routing onto such a link
  answers with reject reason 4 and the high-level client asks again with
  smaller requests instead of ending the read (#1833).
