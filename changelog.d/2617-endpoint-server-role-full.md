---
section: Changed
---
- **Breaking (wire):** An endpoint session with a server role always runs the
  full server: it answers Who-Is, Who-Has and every read service, where it
  rejected all but ReadProperty, and its Device lists the server's services,
  those the read-only server denies included (#2617).
