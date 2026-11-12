---
section: Migration notes
---
- **Event reporting logs (Rust API, #2303):** `FaultDetector::new` takes no
  argument: its `comm_timeout` was never read. A direct caller of
  `handle_get_event_information` no longer gets a warning logged when an
  object's summary doesn't project; `handle_get_event_information_configured`
  returns that object and the reason as `EventInformationFailure::Projection`.
