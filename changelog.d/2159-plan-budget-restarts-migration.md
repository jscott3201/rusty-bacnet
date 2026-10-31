---
section: Migration notes
---
- **MCP plan grants (#2159):** run `bacnet mcp authorize --profile NAME` again for a profile with an `[mcp.plan]` authorized before this release: its plan writes are refused (`plan-budget-unavailable`) until the authorization has started a write count. Rust callers building a `PlanGrant` set `authorized_unix` and call `start_budget` on the workspace.
