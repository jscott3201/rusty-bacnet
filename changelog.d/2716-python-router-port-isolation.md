---
section: Added
---
- **Python API:** `BipRouterPort(..., isolated=True)` keeps a router port
  apart from the router's other isolated ports, as `RouterPort::isolated`
  does in Rust; `port.isolated` reads it back and the repr shows it. The
  default, `False`, routes as before (#2716).
