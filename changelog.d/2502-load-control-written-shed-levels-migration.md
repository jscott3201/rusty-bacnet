---
section: Migration notes
---
- **Load Control storage (Rust API, #2502):** `LoadControlSnapshot` gains
  `shed_levels`, the Shed_Levels elements a peer wrote; a custom
  `LoadControlPersistence` stores it. A file holding one is `RBNLCT02`, which
  older builds refuse; files without one keep their octets. A saved element
  wins over `set_shed_levels`: call `clear_written_shed_levels` first to
  apply yours.
