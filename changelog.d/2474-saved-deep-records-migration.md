---
section: Migration notes
---
- **Saved log records (#2474):** a log snapshot an older build saved with a
  record nested one level deeper than a ReadRange-ACK carries now loads
  without that record, logging a warning; such a record made every page
  that carried it fail to decode. The records older than it each take the
  next number up.
