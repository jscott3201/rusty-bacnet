---
section: Migration notes
---
- **Saved log records (#2474):** a log snapshot an older build saved with a
  record nested one level deeper than a ReadRange-ACK carries now loads
  without that record, logging a warning, instead of being treated as
  damaged; the records older than it each take the next number up.
