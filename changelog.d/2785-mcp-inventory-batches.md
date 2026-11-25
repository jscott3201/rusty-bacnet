---
section: Changed
---
- **MCP:** `bacnet_inventory` and `bacnet_export_223` read a page's objects in batches, one
  ReadPropertyMultiple for up to 16 objects, as the CLI does: a 200-object controller takes about
  28 inventory requests (58 with values) instead of about 201 (#2785).
