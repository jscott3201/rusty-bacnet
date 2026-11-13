---
section: Changed
---
- **Wire:** A confirmed event notification that finds every invoke ID in use
  waits in its destination's lane for one instead of being dropped, so only
  that destination waits; `confirmed_no_invoke_id` no longer counts it
  (#2537).
