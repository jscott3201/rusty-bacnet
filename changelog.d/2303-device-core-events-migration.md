---
section: Migration notes
---
- **Event reporting (Rust API, #2303):** add `bacnet-device-core` (with
  `std`) and import `handle_acknowledge_alarm`, `AcceptedAcknowledgeAlarm`,
  `AcknowledgmentNotificationContext`, `handle_get_event_information`,
  `GetEventInformationBudget` and `ReliabilityChange` from
  `bacnet_device_core::event`, not from `bacnet_server::handlers`,
  `bacnet_server::server` and `bacnet_server::fault_detection`.
