---
section: Migration notes
---
- **CovReporting (Rust API, #2179):** replace `supports_cov() -> true` with
  `fn cov(&self) -> Option<&dyn CovReporting> { Some(self) }` and move
  `cov_increment`, `supports_cov_property` and `cov_reported_properties` into
  `impl CovReporting`. `supports_subscribe_cov_property() -> true` becomes a
  lent `cov()` whose `supports_subscribe_cov()` answers `false`. Delete any
  `set_overridden` override ([details](docs/rust-api.md#capabilities)).
