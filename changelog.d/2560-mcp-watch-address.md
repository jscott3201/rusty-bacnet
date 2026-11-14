---
section: Changed
---
- MCP watches a point at the address its device answers at: `bacnet_watch`
  takes a device named by its address while another claims its instance, a
  subscription stays at the address it began at, and a watched device that
  moves reports `failed`, saying where it went (#2560).
