---
section: Added
---
- bacnet-device-core's `event-enrollment` feature (`alloc` is enough) has Event Enrollment's pass, `event::enrollment::evaluate_event_enrollments_for_delivery`, moved from bacnet-server: it commits in database order, and each delivery carries its committed history for `into_notification()` (#2319).
