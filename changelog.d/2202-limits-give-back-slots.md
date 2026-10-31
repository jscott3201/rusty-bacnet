---
section: Fixed
---
- A request whose wait for its rate-limit slot is cancelled (a withdrawn TUI
  job, an MCP call cancelled or out of budget) now gives the slot and its
  principal token back, so later requests no longer queue behind slots nobody
  used, and the rate meter doesn't count them (#2202).
