---
section: Migration notes
---
- **Event Enrollment (Rust API, #2319):** `EventEnrollmentReliabilityResult::state_change` is an `EventStateChange`, not an `Option`, since a pass commits a Reliability change only with its transition, and `event_type()` returns the `EventType` itself; drop the `Some(..)` and the unwraps.
