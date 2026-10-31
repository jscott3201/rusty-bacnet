---
section: Migration notes
---
- **Timer (wire and Rust API, #2192):** Present_Value reads and takes an
  Unsigned timeout in milliseconds instead of the Enumerated timer state; read
  Timer_State for the state. `TimerObject::start`, `stop`,
  `set_initial_timeout`, `set_update_time` and `set_expiration_time` are gone:
  drive the timer by writing Present_Value, Timer_Running or Timer_State, and
  configure it with `set_pres_value_range` and `set_default_timeout`.
  Initial_Timeout no longer takes writes.
