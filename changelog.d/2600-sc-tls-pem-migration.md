---
section: Migration notes
---
- **SC TLS material (Rust API, #2600):** match `ScTlsConfigError` variants instead of `Error::Encoding` text from `from_der`, or convert with `?` (into `bacnet_types::error::Error` or `OpsError`). Load PEM with `from_pem`/`from_pem_files` instead of `sc_tls::pem`, or decode DER with `rustls::pki_types::pem::PemObject`. Python code matching the old SC messages matches the new ones (see [docs/python-api.md](docs/python-api.md)).
