---
section: Changed
---
- A client's watch polls as often as its settings ask, down to once a
  second (`WatchSettings::poll_every`, 1 s to an hour, the range Python's
  watches already took); the default stays 10 s (#2603).
