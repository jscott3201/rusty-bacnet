---
section: Changed
---
- **Rust API:** A session's full server holds at most 64 of the node's 256
  invoke IDs, so its client keeps the rest; past that a confirmed COV
  notification is skipped with a warning (#2617).
