---
section: Migration notes
---
- **Endpoint wire behaviour (Python, #2617):** an endpoint is now
  discoverable: a scanner-style endpoint answers a global Who-Is with its
  I-Am, so it appears in other tools' device lists. A tool that read its
  Protocol_Services_Supported to learn what it may write must try the write:
  the read-only server denies writes it lists.
