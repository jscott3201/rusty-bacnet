---
section: Added
---
- BACnet/SC TLS configurations load PEM: `ScNodeTlsConfig` and `ScHubTlsConfig` gain `from_pem` and `from_pem_files` with a typed `ScTlsConfigError` naming the file at fault. The CLI and Python bindings use them; `bacnet_ops` re-exports them and adds `OpsError::Tls` (#2600).
