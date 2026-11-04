---
section: Changed
---
- **Binding:** a B/IP transport claims an explicitly requested UDP port, so
  a second bind of it on the host fails at once with an error naming the
  port; `share_wildcard_port` (builders, Python; CLI `--share-port`) keeps
  the old sharing (#2366).
