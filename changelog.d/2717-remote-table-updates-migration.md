---
section: Migration notes
---
- **Router table updates (wire, #2717):** to keep taking routing-table
  updates that reach the router through another router, start it with
  `RouterOptions::new().remote_table_updates()`, or set
  `Options::remote_table_updates` on a `bacnet-router-core` router; a control
  authorizer then sees them with `source_net` set. The Python `BACnetRouter`
  has no such option and refuses them.
