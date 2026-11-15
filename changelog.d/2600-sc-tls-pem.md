---
section: Added
---
- BACnet/SC TLS configurations load PEM: `ScNodeTlsConfig` and `ScHubTlsConfig` gain `from_pem` and `from_pem_files`, with a typed `ScTlsConfigError` that names the file at fault and why; the CLI, Python bindings and `bacnet_ops::Client::sc` load through them (#2600).
