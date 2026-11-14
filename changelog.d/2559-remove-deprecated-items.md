---
section: Removed
---
- **Breaking (Rust API, Python API):** the timestamp-inventing `acknowledge_alarm` is gone; `acknowledge_alarm` (and `acknowledge_alarm_from_device`) now takes the caller's correlation timestamps. `BACnetSuccessFilter::from_legacy_bool` is removed too (#2559).
