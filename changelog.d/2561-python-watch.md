---
section: Added
---
- **Python API:** `Device.watch()` watches points as an async iterator of `WatchUpdate`s: COV renewed before each lifetime ends, polling where COV is refused, and a point given up when its device moves or another takes its address; `close()` or the client's `stop()` cancels it (#2561).
