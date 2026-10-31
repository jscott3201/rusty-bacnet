---
section: Fixed
---
- An MCP listen stream refused at its start, because the session already runs
  its maximum, now frees only the watches set aside for it, never those of
  another waiting stream with the same resources (#2157).
