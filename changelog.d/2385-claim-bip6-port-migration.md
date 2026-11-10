---
section: Migration notes
---
- **Claimed B/IPv6 port (#2385):** two B/IPv6 stacks on one host and port no
  longer both start. Give each its own port, or set `share_wildcard_port`
  (Python's keyword, CLI `--share-port`) on each: both then hear the groups,
  but each unicast reaches one. macOS never let two normal nodes share; there
  a normal node and a foreign device on one port now need it on the later one.
