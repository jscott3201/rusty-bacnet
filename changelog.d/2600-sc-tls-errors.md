---
section: Changed
---
- **Breaking (Rust API):** `ScNodeTlsConfig::from_der` and `ScHubTlsConfig::from_der` return `ScTlsConfigError` instead of `Error::Encoding`, and `bacnet_transport::sc_tls::pem` is no longer public; an SC key file must hold exactly one private key (#2600).
