---
section: Migration notes
---
- **Network Port addresses (Python API, #2710):** reads of properties 418 and 407 on a Network Port give `host_n_port` in place of `tagged`. Read the address from `.value` (`"ip:port"`, `"[v6]:port"`, `"name:port"` or `None`) and the wire bytes from `.octets`.
