---
section: Changed
---
- **MCP:** the last page of a `bacnet_find_object` search that matched
  nothing carries a `note`: it searched only the devices discovered so far,
  by default this network's, and a discovery can reach further (#2596).
