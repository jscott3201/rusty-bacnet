---
section: Fixed
---
- An MCP tool call or resource read needs much less stack in a debug build:
  the server boxes the work each request nests, so its deepest tests need
  under 400 KiB rather than up to 980 KiB, and CI's 1 MiB stack step now
  covers `bacnet-mcp` (#2656).
