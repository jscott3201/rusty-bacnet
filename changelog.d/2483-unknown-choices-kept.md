---
section: Changed
---
- **Wire:** Event_Parameters under tag 7 or 12, an Audit Log record's datum
  under a tag above 2, and event values in primitive form under an
  unassigned tag now decode, kept as received, so a server stores such an
  Event_Parameters write; the CLI and MCP show tag and hex, Python bytes
  (#2483).
