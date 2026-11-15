---
section: Migration notes
---
- **Python Who-Has (#2596):** pass `scope="global"` to `who_has_by_id()`,
  `who_has_by_name()` or `who_has_replies()` to search behind routers as
  before, or a network number for one network. A client on
  `transport="router"` must pass one, as for `who_is()`.
