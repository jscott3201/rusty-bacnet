---
section: Migration notes
---
- **COV notifications (Python API, #2654):** check `notification.kind`, or
  `isinstance`, before reading a single-object field such as
  `monitored_object_identifier` or `values`: a `CovNotificationMultiple` has
  `objects` instead.
