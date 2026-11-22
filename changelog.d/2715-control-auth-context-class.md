---
section: Changed
---
- **Breaking (Rust API):** `ControlAuthContext` gains `class`, the
  `ControlClass` the control is decided and counted under, so an authorizer
  can tell a table query from another network (`InitQuery`) from an update
  (`InitMgmt`) directly (#2715).
