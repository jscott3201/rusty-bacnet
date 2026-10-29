---
section: Added
---
- `bacnet mcp serve` can write when a profile opts in (`mcp_writes`):
  `bacnet_write` and `bacnet_revert` use the TUI's tiers, ledger and audit
  log, now shared in `bacnet-ops`. The client may confirm only low-priority
  writes; a plan grant covers others (#2068).
