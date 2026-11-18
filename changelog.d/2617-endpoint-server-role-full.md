---
section: Changed
---
- **Breaking (wire):** An endpoint session with a server role always runs the
  full server, read-only unless `with_server` gives it a builder: it answers
  Who-Is, Who-Has, every read and COV subscriptions, denies writes, creates
  and deletes, and ignores time synchronization (#2617).
