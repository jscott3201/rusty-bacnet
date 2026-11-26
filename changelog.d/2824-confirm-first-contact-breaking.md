---
section: Changed
---
- **Breaking (Rust API, Python API):** placing a device by instance sends a
  Who-Is and asks each claim (about half a second, 3 s routed; a silent
  claim about 24 s), may fail as `NotConfirmed` or `TableFull`, and never
  moves the handles already following the device (#2824).
