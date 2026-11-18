---
section: Migration notes
---
- **Endpoint stop (Rust API, #2617):** let go of any database guard you hold
  before `EndpointSession::stop()` when storage must hold what the objects
  serve once it returns; a guard still held then leaves the settling to a
  task that runs once it is free.
