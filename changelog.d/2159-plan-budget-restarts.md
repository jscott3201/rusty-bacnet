---
section: Fixed
---
- **Breaking (Rust API):** a plan grant's write budget now holds across `bacnet mcp serve` restarts: it is counted in the profile workspace before each write, and authorizing again starts it fresh. `PlanGrant` gains `authorized_unix` and `start_budget` (#2159).
