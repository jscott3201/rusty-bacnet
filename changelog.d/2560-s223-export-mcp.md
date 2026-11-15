---
section: Changed
---
- `bacnet_export_223` (MCP) reads each object as the inventory's point record, so a binary
  object's texts are read too; a `write_priority` outside 1 to 16 is refused with `priority must
  be 1-16, got N` (#2560).
