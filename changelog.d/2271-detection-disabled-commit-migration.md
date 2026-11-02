---
section: Migration notes
---
- **Event reporting (Rust API, #2271):** a direct
  `EventReporting::commit_event_transition` call on a built-in object with
  Event_Detection_Enable FALSE now fails with `DetectionDisabled` unless its
  target is NORMAL; turn detection on before committing other transitions.
