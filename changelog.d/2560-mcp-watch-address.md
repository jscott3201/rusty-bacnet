---
section: Changed
---
- MCP watches a point at the address its device answers at: `bacnet_watch`
  takes a device named by its address while another claims its instance,
  each row gives the device's `address` (and `uri_ambiguous` when its URI
  names neither), and a subscription stays at the address it began at
  (#2560).
