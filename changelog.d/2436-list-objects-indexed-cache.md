---
section: Fixed
---
- MCP's `bacnet_list_objects` keeps an Object_List read element by element
  for ten minutes after its last progress and, once complete, five minutes
  from then, so a slow read finishes and is reused rather than starting
  over (#2436).
