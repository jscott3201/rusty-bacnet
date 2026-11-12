---
section: Added
---
- **Wire:** A Load Control with storage (Python: `storage_path`) keeps the
  Shed_Levels elements a peer wrote across a restart, where they win over the
  application's start-up levels until `set_load_control_shed_levels_local`
  or `clear_written_shed_levels` drops them (#2502).
