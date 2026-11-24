---
section: Fixed
---
- A source-audited request no longer waits for the node's database in its
  caller's task, so a caller parked in `select!`, `join!` or
  `buffer_unordered` no longer holds the database or a slot and stalls the
  node's other requests (#2755).
