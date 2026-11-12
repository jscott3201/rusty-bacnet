---
section: Added
---
- **Python API:** A running Load Control's Shed_Levels, Full_Duty_Baseline and
  State_Description change through `BACnetServer.set_load_control_*_local`
  (Rust: `set_load_control_*_local`), and `add_load_control` takes
  `storage_path` to keep a request across a restart (#2497).
