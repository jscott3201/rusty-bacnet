---
section: Migration notes
---
- **B/IPv6 configured BBMD (Rust API, #2365):** write `Bip6ForeignDeviceConfig { bbmd: (ip, port).into(), .. }` where you wrote `bbmd_ip: ip, bbmd_port: port`, or `"host:port".parse()?` for a host name; read the port back with `config.bbmd.port()` and the address with `config.bbmd.address()`.
