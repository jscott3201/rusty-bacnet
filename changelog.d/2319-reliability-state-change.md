---
section: Changed
---
- **Breaking (Rust API):** an Event Enrollment Reliability result in `bacnet_server::event_enrollment`'s report always carries its state change, so `EventEnrollmentReliabilityResult::state_change` and `event_type()` are no longer `Option`s (#2319).
