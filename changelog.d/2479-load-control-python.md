---
section: Added
---
- **Python API:** `BACnetServer.add_load_control` takes `shed_levels`, `shed`
  and `stop` callables for the loads (a new `ShedRequest` in, how the loads
  stand out), `full_duty_baseline`, `state_description` and
  `agreed_duty_window`, so a Load Control built from Python sheds (#2479).
