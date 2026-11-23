---
section: Added
---
- TUI/CLI: an SC profile asks for unlimited hub recovery with
  `reconnect_max_retries = "unlimited"`; the connection view shows the rounds
  and `0` as no recovery, and a reconnect value out of range is a profile
  error naming its key (#2708).
