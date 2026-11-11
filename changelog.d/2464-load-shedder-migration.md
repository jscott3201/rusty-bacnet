---
section: Migration notes
---
- **Load Control (Rust API, #2464):** `LoadControlObject::set_actual_shed_level`
  is gone. Implement `bacnet_objects::load_control::LoadShedder` for the loads
  and install it with `set_shedder`: its `shed` returns a `ShedStatus` with the
  progress and the expected and actual levels, and its `stop` ends shedding.
  Set Shed_Levels and their descriptions with `set_shed_levels`.
