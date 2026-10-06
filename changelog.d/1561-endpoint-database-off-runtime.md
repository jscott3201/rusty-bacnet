---
section: Fixed
---
- Every endpoint session holder of the object database (the session, the
  server role's responder, the source Audit runtime and the Number task) now
  lets go of it off the runtime, so whichever goes last, durable objects'
  final saves don't hold a Tokio worker (#1561).
