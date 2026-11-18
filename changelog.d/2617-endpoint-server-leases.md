---
section: Changed
---
- **Rust API:** A session's full server holds at most 64 of the node's 256
  invoke IDs, so its client keeps the rest. Past that a confirmed COV
  notification waits for one to come back, as the endpoint's own COV
  reporting did; one an in-flight limit refused first waits for the next
  change (#2617).
