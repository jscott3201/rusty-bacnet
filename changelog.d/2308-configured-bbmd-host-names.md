---
section: Added
---
- A foreign device's configured BBMD may be a host name (`ForeignBbmd`, the CLI's `--foreign-bbmd host[:port]`, Python's `ForeignDevice("host:port")`); one that doesn't resolve still starts, its failure counted and retried every 30 s (#2308).
