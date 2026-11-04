---
section: Migration notes
---
- **Claimed B/IP port (#2366):** a device or tool that shared its port, say
  47808, with another stack on one host now fails to start. Give each stack
  its own port or address (`share_port_by_address`), or set
  `share_wildcard_port` (CLI `--share-port`) on every stack that shares it;
  unicast then reaches only one of them. Exhaustive `ServerConfig` and
  `ClientConfig` literals need `share_wildcard_port: false`.
