---
section: Changed
---
- **Wire:** Event values under tag 7, 12 or 23 and up (20 is still refused),
  and log record choices under a tag their CHOICE doesn't assign, now decode,
  kept as received: the client acknowledges such a confirmed notification,
  not rejects it; the CLI and MCP show tag and hex, Python bytes (#2474).
