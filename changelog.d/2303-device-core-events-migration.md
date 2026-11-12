---
section: Migration notes
---
- **Event reporting (Rust API, #2303):** import `handle_acknowledge_alarm`,
  `AcceptedAcknowledgeAlarm`, `AcknowledgmentNotificationContext`,
  `handle_get_event_information`, `GetEventInformationBudget` and
  `ReliabilityChange` from `bacnet_device_core::event` instead of
  `bacnet_server::handlers`, `bacnet_server::server` and
  `bacnet_server::fault_detection`. `FaultDetector::new` takes no argument:
  its `comm_timeout` was never read and is gone.
