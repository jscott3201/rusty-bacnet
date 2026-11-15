---
section: Changed
---
- **Breaking (Rust API):** `from_der` on both SC TLS configurations returns `ScTlsConfigError`, not `Error::Encoding`, and `sc_tls::pem` is private. A key file must hold exactly one key in PEM well formed throughout, for connects and `bacnet sc-check` alike (#2600).
