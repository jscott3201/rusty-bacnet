---
section: Changed
---
- **Breaking (Rust API, Python API):** a first contact by instance costs one
  identity read per address claiming it (none once confirmed, or for a row
  registered by hand); a claim that doesn't answer as the device fails as
  `NotConfirmed`, several as `Ambiguous`, rather than sending there (#2824).
