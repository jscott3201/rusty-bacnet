---
section: Added
---
- **Breaking (Rust API):** The Audit Log keeps a peer-written Description in
  its storage, saved before the reply; a saved one wins over
  `set_description` until `clear_written_description` (#2407). **Python
  API:** `add_audit_log`'s `storage_path` keeps it too.
