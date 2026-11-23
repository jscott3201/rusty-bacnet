---
section: Added
---
- **Breaking (Rust API):** The server builders' `device_storage_path` and the
  endpoint session's `with_device_storage_path` keep what peers write to the
  Device they build, a Description and with `clock_zone_writable` the zone,
  in a file that survives a restart (#2730).
