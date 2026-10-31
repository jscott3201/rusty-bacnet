---
section: Fixed
---
- **Breaking (CLI):** a plan grant's write budget now holds across `bacnet mcp serve` restarts: it is counted in the profile workspace before each write, and `bacnet mcp authorize` starts it fresh, so a plan authorized by an earlier release must be authorized again (#2159).
