---
section: Migration notes
---
- **Log reporting settings (Rust API, #1952):** `LogSnapshot` gains
  `reporting`, a `LogReportingSettings`; a custom `LogPersistence` must store
  it, or a peer's write is answered but lost. A saved write wins over its
  setter: call `clear_written_reporting` first (Python:
  `clear_written_reporting=True`) to apply yours. Version 1 files still load.
