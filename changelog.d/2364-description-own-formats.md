---
section: Added
---
- **Breaking (Rust API):** The three logs, Notification Class,
  Notification Forwarder and Access Rights keep a peer-written Description
  in their own storage files, saved before the reply; a saved one wins
  over `set_description` (#2364). **Python API:** the same `storage_path`.
