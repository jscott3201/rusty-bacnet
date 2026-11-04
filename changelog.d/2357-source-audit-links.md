---
section: Added
---
- **Rust and Python API:** an endpoint session's source Audit Reporter now works on
  every link, not only IPv4 B/IP: B/IPv6, SC and MS/TP sessions take Device
  bindings in their own link's format, and `ScEndpoint` and `MstpEndpoint` take
  the source Reporter keywords and `add_audit_reporter` (#2357).
