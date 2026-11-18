---
section: Migration notes
---
- **Endpoint stop (Rust API, #2617):** let go of every database guard before
  `EndpointSession::stop()` where storage must be settled when it returns.
  With one held, a task settles the objects once it is free; a runtime shut
  down first drops that task, and storage is put back only as the objects
  drop. After stop, let a runtime turn pass before `Arc::try_unwrap` of the
  database: the server lets go of its handle from a task.
