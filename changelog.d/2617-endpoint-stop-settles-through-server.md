---
section: Changed
---
- **Breaking (Rust API):** An endpoint session's stop settles its objects'
  saves as a standalone server's stop does: a database guard held at that
  moment hands the settling to a task, so the stop may return before
  storage is settled
  ([details](docs/rust-api.md#endpoint-shutdown-and-the-object-database),
  #2617).
