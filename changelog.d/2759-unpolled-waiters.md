---
section: Fixed
---
- A call polled once and then left unpolled no longer holds what other
  callers wait for: a client's routed path, routing-table query or network
  number question, or an endpoint session's database or policy counters
  (#2759).
