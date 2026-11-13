---
section: Migration notes
---
- **Event Enrollment (Rust API, #2319):** `bacnet_server::event_enrollment::encode_out_of_range_params` and its four siblings are gone: they wrote the evaluator's private legacy layout, not a form the standard defines. The report types (`EventEnrollmentEvaluationReport` and the rest) keep their `bacnet_server::event_enrollment` paths and are bacnet-device-core's, re-exported; a host without the server takes them from `bacnet_device_core::event::enrollment` (add `bacnet-device-core` with `event-enrollment`; `alloc` is enough).
