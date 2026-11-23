---
section: Migration notes
---
- **SC reconnect delay (Rust, Python, TUI/CLI profiles, #2708):** lower an
  `ScReconnectConfig.max_delay_ms`, `ScReconnect(max_delay_ms=...)` or profile
  `reconnect_max_delay_ms` above 600,000 to 600,000 or less (the default is
  600,000); a larger value is now refused when the client, endpoint or
  profile is checked.
