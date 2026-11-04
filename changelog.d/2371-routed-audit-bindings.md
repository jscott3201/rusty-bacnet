---
section: Added
---
- **Rust and Python API:** an endpoint session's source Audit Reporter can bind
  a recipient Device behind a router, by network number and MAC (`"2:0x05"` in
  Python), on every link; records go through the router, which the session
  learns from traffic and router announcements (#2371).
