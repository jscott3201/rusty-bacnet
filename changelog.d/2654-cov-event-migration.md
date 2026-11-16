---
section: Migration notes
---
- **COV notifications (Rust API, #2654):** the stream's item is a `CovEvent`:
  `let n: ReceivedCOVNotification = rx.recv().await?` becomes
  `if let CovEvent::Object(n) = rx.recv().await? { .. }`, and
  `CovEvent::Multiple` carries a `ReceivedCOVNotificationMultiple`. An ack
  policy reads `event.delivery()` and the other shared fields through
  methods; pass the event to `Station::is_source_of`.
