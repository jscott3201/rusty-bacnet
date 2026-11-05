---
section: Migration notes
---
- **Snapshot formats (Rust API, #2364):** `LogReportingSettings`,
  `NotificationClassSnapshot`, `ForwarderSnapshot` and
  `AccessRightsSnapshot` gain a `description` field. A file holding one is
  log schema version 3, `RBNNCL02`, `RBNFWD02` or `RBNACR02`, which older
  builds refuse; files without one keep their octets.
- **Durable log size (#2364):** a durable log's largest Buffer_Size drops
  slightly, to leave room for a Description: Trend Log 828,251, Event Log
  178,426, Trend Log Multiple 15,848. A log configured above the new limit
  but within the old one (for example a Trend Log at 828,300) is now
  refused when it opens; lower its Buffer_Size.
