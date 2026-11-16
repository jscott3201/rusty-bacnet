---
section: Changed
---
- **Breaking (Rust API, Python API):** `acknowledge_alarm_request` is now
  `acknowledge_alarm` on `BACnetClient`, bacnet-ops `Destination` and Python.
  The client's `acknowledge_alarm_request_from_device` is gone: send
  `acknowledge_alarm` to the station `resolve_device` gives (#2559, #2588).
