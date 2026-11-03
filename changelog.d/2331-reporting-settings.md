---
section: Added
---
- Analog, binary and multi-state objects built `with_reporting_persistence`
  keep the reporting settings a peer writes, saved before the reply; a saved
  one wins over its setter until `clear_written_reporting` (#2331).
  **Python API:** `storage_path` and `clear_written_reporting`.
