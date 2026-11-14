---
section: Changed
---
- **Breaking (Python API):** `who_has_by_id()`, `who_has_by_name()` and
  `who_has_replies()` take `scope="local"` (the default), `"global"` or a
  network number, as `who_is()` does, and with no scope send a local
  broadcast instead of a global one (#2596).
