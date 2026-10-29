---
section: Changed
---
- `bacnet_types::text` holds the one table of hidden characters and the
  line-break check that the CLI, the TUI, the MCP envelopes and the PICS use
  to clean device text, so they can't drift apart (#2148).
