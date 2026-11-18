---
section: Changed
---
- **Rust API:** A session's full server holds at most 64 of the node's 256
  invoke IDs, so its client keeps the rest; past that a confirmed COV
  notification waits for one of the server's IDs to come back, as the
  endpoint's own COV reporting did (#2617, #2661).
