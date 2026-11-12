---
section: Added
---
- **Wire:** A Load Control with storage keeps the Shed_Levels elements a peer
  wrote across a restart, where they win over the application's start-up
  levels until `set_load_control_shed_levels_local` drops them (#2502).
  **Python API:** the same `storage_path`.
