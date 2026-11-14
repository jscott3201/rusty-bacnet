---
section: Changed
---
- An address whose I-Am names another instance is announced as `Lost` for
  the old instance and then `Discovered` for the new, so device-event
  subscribers (the TUI, MCP's `bacnet://devices`) drop the old one (#2560).
