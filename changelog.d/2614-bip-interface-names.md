---
section: Added
---
- B/IP Python clients, servers, endpoints and router ports accept an OS interface name. Rust callers can use `bacnet_transport::bip::resolve_interface_ipv4`. A name with several usable IPv4 addresses reports the choices and accepts an explicit address (#2614).
