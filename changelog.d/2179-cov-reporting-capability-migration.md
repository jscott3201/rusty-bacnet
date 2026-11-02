---
section: Migration notes
---
- **CovReporting (Rust API, #2179):** lend `cov()` in place of
  `supports_cov() -> true` and move the COV hooks into `impl CovReporting`;
  a property-only object answers `supports_subscribe_cov()` with `false`.
  `cov_reported_properties` returns `None` for the type's Table 13-1 row. A
  custom Life Safety object narrows its own properties. Concrete calls need
  `use bacnet_objects::traits::CovReporting` ([details](docs/rust-api.md#capabilities)).
