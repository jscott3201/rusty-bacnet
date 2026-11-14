---
section: Migration notes
---
- **Event notification counters (Rust and Python API, #2537):** read
  `confirmed_waited_for_invoke_id` where you read `confirmed_no_invoke_id`.
  It counts confirmed notifications that waited for an invoke ID, since none
  is dropped for want of one any more; no counter replaces the old drops.
