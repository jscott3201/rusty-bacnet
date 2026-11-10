---
section: Migration notes
---
- **Claimed B/IPv6 port (#2385):** two B/IPv6 stacks on one host and port,
  say 47808, no longer both start. Give each its own port, or set
  `share_wildcard_port` (`Bip6Transport::set_share_wildcard_port`, Python's
  keyword with `transport="ipv6"`, CLI `--share-port`) on every stack that
  shares it; both then hear the multicast groups, but each unicast reaches
  only one of them.
