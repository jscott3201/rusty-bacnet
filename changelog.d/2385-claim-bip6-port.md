---
section: Changed
---
- **Binding:** a B/IPv6 transport claims an explicitly requested UDP port,
  as B/IP does, so a second bind of it on the host fails at once with an
  error naming the port; `share_wildcard_port` (transport, client builder,
  Python; CLI `--share-port`) keeps the old sharing (#2385).
