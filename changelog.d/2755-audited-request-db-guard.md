---
section: Fixed
---
- A source-audited request waits for the node's database in a task the
  session owns, so a caller parked in `select!`, `join!` or
  `buffer_unordered` no longer holds the database or a slot and stalls the
  node's other requests (#2755).
