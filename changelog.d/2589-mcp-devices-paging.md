---
section: Fixed
---
- MCP: `bacnet://devices` pages by instance and address, so a page boundary between two addresses that claim one instance no longer skips the second; the page URI is now `bacnet://devices/after/{instance}/{network}/{mac}`, which `next` names (#2589).
