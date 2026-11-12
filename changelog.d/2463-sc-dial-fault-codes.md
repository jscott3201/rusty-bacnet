---
section: Changed
---
- **Breaking (Rust and Python API):** an SC dial with no route to the hub, and
  an upgrade answer with a bad WebSocket header or one that is not HTTP/1.1,
  get their own error kinds and codes; a connect NAK's message names its error
  class and code (#2463).
