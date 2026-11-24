---
section: Fixed
---
- A call left unpolled while it waits its turn no longer holds up others: a
  client's routed path, routing-table query or network number question, and
  an endpoint's database and counters. Routed requests keep their order when
  polled; on a multi-thread runtime a slow one can be passed (#2759).
