---
section: Added
---
- bacnet-device-core's `event-enrollment` feature has Event Enrollment's evaluation pass, `event::enrollment::evaluate_event_enrollments_for_delivery`, with its report and delivery types, moved from bacnet-server, which drives it unchanged; it needs only `alloc` (#2319).
