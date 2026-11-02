---
section: Migration notes
---
- **Event reporting (Rust API, #2271):** `EventReporting::commit_event_transition`
  on a built-in object with Event_Detection_Enable FALSE now fails with
  `DetectionDisabled`. Binary Output and Multi-state Output start with
  detection off, so turn it on before committing.
