---
section: Migration notes
---
- **Log reporting settings (Rust API, #1952):** `LogSnapshot` gains
  `reporting`, a `LogReportingSettings`; a custom `LogPersistence` stores it.
  Once a peer's write is saved, `set_notification_class`, `set_event_enable`,
  `set_notify_type` and the Trend Log's COV setters keep it: call
  `clear_written_reporting` first (Python: `clear_written_reporting=True` on
  `add_trend_log`) to apply yours. Version 1 files still load.
