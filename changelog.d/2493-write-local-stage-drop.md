---
section: Fixed
---
- A `write_local`, a Command, Channel or Timer run's write, or a `purge_audit_log` dropped or aborted during a durable save now releases its stage at once, not about 10 s later; `purge_audit_log` now fails with an encoding error outside a Tokio runtime (#2493).
