---
section: Migration notes
---
- **Load Control storage (Rust API, #2502):** `LoadControlSnapshot` gains
  `shed_levels`, the Shed_Levels elements a peer wrote, and is
  `#[non_exhaustive]`: build one from `LoadControlSnapshot::default()` and set
  its fields; a custom `LoadControlPersistence` stores the new one. A file
  holding written levels is `RBNLCT02`, which older builds refuse; files
  without keep their octets. A saved element wins over `set_shed_levels`:
  call `clear_written_shed_levels` first to apply yours.
