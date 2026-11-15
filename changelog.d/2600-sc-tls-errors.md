---
section: Changed
---
- **Breaking (Rust API):** SC TLS `from_der` returns `ScTlsConfigError`, not `Error::Encoding`; `sc_tls::pem` is private; `ScInputErrorKind` gains `MultiplePrivateKeys` and `UnsupportedPrivateKey`. A key file must hold one key in well-formed PEM, for connects and `bacnet sc-check` alike (#2600).
