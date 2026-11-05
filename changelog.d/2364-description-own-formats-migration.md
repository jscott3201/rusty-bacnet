---
section: Migration notes
---
- **Snapshot formats (Rust API, #2364):** `LogReportingSettings`,
  `NotificationClassSnapshot`, `ForwarderSnapshot` and
  `AccessRightsSnapshot` gain a `description` field. A file holding one is
  log schema version 3, `RBNNCL02`, `RBNFWD02` or `RBNACR02`, which older
  builds refuse; files without one keep their octets. A durable log's
  largest Buffer_Size drops slightly (Trend Log 828,251, Event Log 178,426,
  Trend Log Multiple 15,848) to leave room for it.
