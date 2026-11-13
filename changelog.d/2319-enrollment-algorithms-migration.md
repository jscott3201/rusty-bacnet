---
section: Migration notes
---
- **Event Enrollment parameter encoders (Rust API, #2319):** `bacnet_server::event_enrollment::encode_out_of_range_params` and its four siblings moved to `bacnet_device_core::event::enrollment::algorithms`; update the import.
