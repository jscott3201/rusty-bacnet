---
section: Fixed
---
- Saving the session ledger, a profile or the MCP plan's write count now syncs the workspace directory on Unix, where the filesystem supports it, so a power loss can't undo a save that reported success (#2159).
