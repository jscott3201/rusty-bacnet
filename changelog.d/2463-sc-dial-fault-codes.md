---
section: Changed
---
- **Breaking (Rust and Python API):** an SC dial with no route to the hub, and
  an upgrade answer with a bad WebSocket header, a malformed one or one with an
  unusable status line, get their own error kinds and codes; a NAK's message
  and log line name its error class and code (#2463).
