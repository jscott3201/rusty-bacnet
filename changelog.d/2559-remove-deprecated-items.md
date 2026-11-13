---
section: Removed
---
- **Breaking (Rust API, Python API):** `BACnetClient::acknowledge_alarm` and the Python `acknowledge_alarm` (both made up zero timestamps) and `BACnetSuccessFilter::from_legacy_bool` are gone; use `acknowledge_alarm_request` and the named `BACnetSuccessFilter` constants (#2559).
