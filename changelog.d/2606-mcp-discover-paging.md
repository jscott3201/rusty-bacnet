---
section: Fixed
---
- MCP: `bacnet_discover` pages by device, as `bacnet://devices` does: a later page starts after the last device listed, so a device heard or lost between pages no longer repeats or skips another (#2606).
