---
section: Migration notes
---
- **Alert Enrollment (Rust API, #2331):** assign Event_Enable,
  Notification_Class and Event_Detection_Enable through
  `set_event_enable`, `set_notification_class` and
  `set_event_detection_enable` instead of the fields, and read them with
  `read_property`.
