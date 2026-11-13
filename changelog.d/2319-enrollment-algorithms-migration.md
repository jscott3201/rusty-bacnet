---
section: Migration notes
---
- **Event Enrollment parameter encoders (Rust API, #2319):** `bacnet_server::event_enrollment::encode_out_of_range_params` and its four siblings are gone from bacnet-server: they write the evaluator's private legacy layout, not a form the standard defines, and moved with the algorithms into bacnet-device-core's internal `event::enrollment::algorithms`, undocumented. A caller that still needs them adds `bacnet-device-core` (with `std`) as a dependency and imports them from there.
