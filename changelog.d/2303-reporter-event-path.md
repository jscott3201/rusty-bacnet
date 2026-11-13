---
section: Added
---
- **Breaking (Rust API):** bacnet-device-core's `Reporter` reports intrinsic events when built with `ReportConfig::with_sweep`: event notifications through its outbox and confirmed table, AcknowledgeAlarm, GetEventInformation and a local `acknowledge_local` (#2303).
