---
section: Fixed
---
- **Breaking (MCP):** `bacnet://devices` pages by instance and address, so a page boundary between two addresses claiming one instance no longer skips one; the page URI is `bacnet://devices/after/{instance}/{network}/{mac}` (#2589).
