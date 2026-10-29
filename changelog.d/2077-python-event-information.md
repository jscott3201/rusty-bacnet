---
section: Changed
---
- **Breaking (Rust API, Python API):** `get_event_information` returns the
  decoded acknowledgment instead of raw octets: a `GetEventInformationAck` in
  Rust, and `{"summaries": [...], "more_events": ...}` with named states,
  `BitString` transitions and timestamps in Python (#2077).
