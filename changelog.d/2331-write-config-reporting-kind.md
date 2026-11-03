---
section: Changed
---
- **Breaking (Rust API):** `WriteConfigSnapshot` gains a `Reporting`
  variant, stored as kind 4 of the `RBNWCF01` format, and it and
  `ReportingWriteConfig` are `#[non_exhaustive]` (#2331).
