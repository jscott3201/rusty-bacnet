---
section: Security
---
- **Breaking (wire):** an I-Am the server did not ask for no longer binds a
  device or moves a binding. Only an answer to the server's own Who-Is binds;
  another address claiming a bound device starts a check of both, and the
  binding moves only if the old address stays silent (#2825).
