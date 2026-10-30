---
section: Fixed
---
- A watched point (MCP watch tools and resources, the TUI) keeps the value of
  a COV notification that arrives before the point's first read is applied,
  instead of showing the older read until the next change (#2169).
