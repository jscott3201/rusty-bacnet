---
section: Added
---
- **Breaking (Rust API):** The objects that keep written reporting
  settings, and Timer and Channel, keep a peer-written Description in the
  same file, saved before the reply; a saved one wins over
  `set_description` until the object's clear (#2364). **Python API:** the
  same `storage_path`.
