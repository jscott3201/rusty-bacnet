---
section: Migration notes
---
- **Custom intrinsic reporters (Rust API, #2303):** the server wakes early
  only for a due instant already pending at its last sweep, so a custom
  `EventReporting` that seeds one less than a second after a write is served
  by the next periodic sweep, up to a second late. The built-in detectors
  seed whole seconds of Time_Delay.
