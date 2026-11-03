---
section: Added
---
- Analog, binary and multi-state objects built `with_reporting_persistence`
  keep the reporting settings a peer writes, saved before the write is
  answered; a saved one wins over its setter until `clear_written_reporting`
  (#2331). **Python API:** their `add_*` builders take `storage_path`.
